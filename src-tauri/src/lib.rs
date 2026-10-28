pub mod engine;
mod recorder;
pub mod store;
pub mod summarize;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use whisper_rs::WhisperContext;

use engine::{ProcessOptions, Stage, Transcript};
use store::{Recording, Settings, Status, Store};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

struct AppState {
    store: Store,
    home: PathBuf,
    /// 1.6GB 모델을 녹음마다 다시 읽지 않도록 경로와 함께 들고 있는다
    whisper: Mutex<Option<(PathBuf, Arc<WhisperContext>)>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// GPU와 CPU를 다 쓰는 작업이라 한 번에 하나만 처리한다
    pipeline: tokio::sync::Mutex<()>,
    /// 녹음은 한 번에 하나만. 녹음 id와 녹음 스레드 핸들
    recorder: Mutex<Option<(String, recorder::ActiveRecording)>>,
    /// 요약 중인 녹음 id → 취소 신호
    summaries: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
}

impl AppState {
    fn whisper(&self, path: &Path) -> eyre::Result<Arc<WhisperContext>> {
        let mut slot = self.whisper.lock().unwrap();
        if let Some((loaded, ctx)) = slot.as_ref() {
            if loaded == path {
                return Ok(ctx.clone());
            }
        }
        let ctx = Arc::new(engine::load_whisper(path)?);
        *slot = Some((path.to_path_buf(), ctx.clone()));
        Ok(ctx)
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    id: String,
    stage: Stage,
    progress: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingDetail {
    recording: Recording,
    audio_path: PathBuf,
    transcript: Option<Transcript>,
    summary: Option<summarize::Summary>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRequest {
    id: String,
    num_speakers: Option<i32>,
    languages: Vec<String>,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.store.get_settings(&state.home)
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: Settings) -> CmdResult<()> {
    state.store.save_settings(&settings).map_err(err)
}

#[tauri::command]
fn list_recordings(state: State<AppState>) -> CmdResult<Vec<Recording>> {
    state.store.list().map_err(err)
}

#[tauri::command]
fn import_recordings(state: State<AppState>, paths: Vec<PathBuf>) -> CmdResult<Vec<Recording>> {
    paths.iter().map(|p| state.store.import(p).map_err(err)).collect()
}

#[tauri::command]
fn get_recording(state: State<AppState>, id: String) -> CmdResult<RecordingDetail> {
    let recording = state.store.get(&id).map_err(err)?;
    Ok(RecordingDetail {
        audio_path: state.store.audio_path(&recording).map_err(err)?,
        transcript: state.store.get_transcript(&id).map_err(err)?,
        summary: state.store.get_summary(&id).map_err(err)?,
        recording,
    })
}

#[tauri::command]
fn update_recording(state: State<AppState>, recording: Recording) -> CmdResult<()> {
    // 상태와 오류는 처리 파이프라인만 바꾼다. 프론트가 보낸 값으로 덮어쓰지 않는다
    let current = state.store.get(&recording.id).map_err(err)?;
    state
        .store
        .save(&Recording {
            status: current.status,
            error: current.error,
            duration: current.duration,
            audio_file: current.audio_file,
            ..recording
        })
        .map_err(err)
}

#[tauri::command]
fn delete_recording(state: State<AppState>, id: String) -> CmdResult<()> {
    if state.recorder.lock().unwrap().as_ref().is_some_and(|(r, _)| r == &id) {
        return Err("녹음 중인 항목은 삭제할 수 없습니다. 먼저 녹음을 멈춰 주세요".into());
    }
    if state.cancels.lock().unwrap().contains_key(&id) {
        return Err("처리 중인 녹음은 삭제할 수 없습니다. 먼저 취소해 주세요".into());
    }
    state.store.delete(&id).map_err(err)
}

#[tauri::command]
fn cancel_processing(state: State<AppState>, id: String) {
    if let Some(flag) = state.cancels.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn list_input_devices() -> CmdResult<Vec<recorder::InputDevice>> {
    recorder::list_input_devices().map_err(err)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LevelEvent {
    id: String,
    /// 트랙 순서(마이크, 시스템 소리)대로 0..1
    levels: Vec<f32>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InterruptedEvent {
    id: String,
    reason: String,
}

// 마이크를 열고 녹음 스레드를 합치는 동안 메인 스레드(화면)가 멈추지 않도록 async로 둔다
#[tauri::command]
async fn start_recording(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Recording> {
    let mut slot = state.recorder.lock().unwrap();
    if slot.is_some() {
        return Err("이미 녹음 중입니다".into());
    }
    let settings = state.store.get_settings(&state.home);
    let mut sources = vec![("mic.wav", recorder::Source::Mic(settings.input_device_id.clone()))];
    if settings.capture_system_audio {
        sources.push(("system.wav", recorder::Source::System));
    }
    let title = format!("녹음 {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
    let source_name = if settings.capture_system_audio { "마이크 + 시스템 소리" } else { "마이크 녹음" };
    let recording = state
        .store
        .create_for_recording(
            title,
            source_name.to_string(),
            sources.iter().map(|(f, _)| f.to_string()).collect(),
        )
        .map_err(err)?;
    let tracks = state
        .store
        .track_paths(&recording)
        .map_err(err)?
        .into_iter()
        .zip(sources)
        .map(|(path, (_, source))| recorder::Track { source, path })
        .collect();

    let id = recording.id.clone();
    let level_app = app.clone();
    let interrupted_id = recording.id.clone();
    let started = recorder::start(
        tracks,
        move |levels| {
            let _ = level_app.emit("recording-level", LevelEvent { id: id.clone(), levels });
        },
        move |reason| {
            let _ = app.emit("recording-interrupted", InterruptedEvent { id: interrupted_id, reason });
        },
    );
    match started {
        Ok(active) => {
            *slot = Some((recording.id.clone(), active));
            Ok(recording)
        }
        Err(e) => {
            // 장치를 못 열었으면 빈 항목을 남기지 않는다
            let _ = state.store.delete(&recording.id);
            Err(format!("{e:#}"))
        }
    }
}

/// 트랙을 섞어 재생과 전사에 쓸 audio.wav를 만든다. 녹음 길이(초)를 돌려준다
fn finalize_tracks(store: &Store, recording: &Recording) -> eyre::Result<f32> {
    let tracks = store.track_paths(recording)?;
    let existing: Vec<PathBuf> = tracks.into_iter().filter(|p| p.exists()).collect();
    engine::audio::mix_wav_tracks(&existing, &store.audio_path(recording)?)
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> CmdResult<Recording> {
    let state = app.state::<AppState>();
    let (id, active) = state.recorder.lock().unwrap().take().ok_or("녹음 중이 아닙니다")?;
    let result = active.stop();

    let worker_app = app.clone();
    let worker_id = id.clone();
    // 1시간 녹음이면 트랙을 섞는 데 몇 초 걸리므로 블로킹 스레드에서 한다
    let mixed = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let recording = state.store.get(&worker_id)?;
        finalize_tracks(&state.store, &recording)
    })
    .await
    .map_err(err)?;

    let mut recording = state.store.get(&id).map_err(err)?;
    recording.status = Status::New;
    recording.error = match (&result, &mixed) {
        (Err(e), _) => Some(format!("녹음을 마무리하지 못했습니다: {e:#}")),
        (_, Err(e)) => Some(format!("트랙을 합치지 못했습니다: {e:#}")),
        (Ok(f), Ok(_)) => f
            .interrupted
            .as_ref()
            .map(|e| format!("녹음 장치 오류로 중간에 멈췄습니다. 그 전까지는 저장됐습니다: {e}")),
    };
    recording.duration = mixed.ok();
    state.store.save(&recording).map_err(err)?;
    Ok(recording)
}

#[tauri::command]
async fn summary_providers() -> Vec<summarize::ProviderStatus> {
    // 로그인 셸을 띄워 PATH를 알아내므로 메인 스레드를 막지 않게 async로 둔다
    tauri::async_runtime::spawn_blocking(summarize::statuses).await.unwrap_or_default()
}

#[tauri::command]
fn default_summary_prompt() -> &'static str {
    summarize::DEFAULT_PROMPT
}

#[tauri::command]
async fn summarize_recording(
    app: AppHandle,
    id: String,
    provider: summarize::Provider,
) -> CmdResult<summarize::Summary> {
    let state = app.state::<AppState>();
    let recording = state.store.get(&id).map_err(err)?;
    let transcript = state
        .store
        .get_transcript(&id)
        .map_err(err)?
        .ok_or("먼저 전사를 끝내 주세요")?;
    let mut settings = state.store.get_settings(&state.home);
    let instructions = settings.summary_prompt.clone().unwrap_or_else(|| summarize::DEFAULT_PROMPT.to_string());
    let prompt = summarize::build_prompt(&instructions, &recording, &transcript);

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    {
        let mut summaries = state.summaries.lock().unwrap();
        if summaries.contains_key(&id) {
            return Err("이미 요약하는 중입니다".into());
        }
        summaries.insert(id.clone(), cancel_tx);
    }
    let result = summarize::run(provider, prompt, cancel_rx).await;
    state.summaries.lock().unwrap().remove(&id);

    let markdown = result.map_err(|e| format!("{e:#}"))?;
    let summary = summarize::Summary {
        provider,
        created_at: chrono::Utc::now(),
        markdown,
    };
    state.store.save_summary(&id, &summary).map_err(err)?;
    // 다음 요약에서 같은 CLI를 미리 고르도록 기억한다
    settings.summary_provider = Some(provider);
    let _ = state.store.save_settings(&settings);
    Ok(summary)
}

#[tauri::command]
fn cancel_summary(state: State<AppState>, id: String) {
    if let Some(tx) = state.summaries.lock().unwrap().remove(&id) {
        let _ = tx.send(());
    }
}

#[tauri::command]
async fn process_recording(app: AppHandle, request: ProcessRequest) -> CmdResult<Transcript> {
    let state = app.state::<AppState>();
    let id = request.id.clone();

    let mut recording = state.store.get(&id).map_err(err)?;
    recording.status = Status::Processing;
    recording.error = None;
    recording.num_speakers = request.num_speakers;
    recording.languages = request.languages.clone();
    state.store.save(&recording).map_err(err)?;

    let cancel = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id.clone(), cancel.clone());

    let _guard = state.pipeline.lock().await;
    let worker_app = app.clone();
    let worker_id = id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let settings = state.store.get_settings(&state.home);
        let recording = state.store.get(&worker_id)?;
        let audio = state.store.audio_path(&recording)?;
        let whisper = state.whisper(&settings.models.whisper)?;

        let emitter = worker_app.clone();
        let event_id = worker_id.clone();
        engine::process(
            &audio,
            &whisper,
            &settings.models,
            &ProcessOptions {
                num_speakers: request.num_speakers,
                languages: request.languages,
            },
            cancel,
            move |stage, progress| {
                let _ = emitter.emit(
                    "progress",
                    ProgressEvent {
                        id: event_id.clone(),
                        stage,
                        progress,
                    },
                );
            },
        )
    })
    .await
    .map_err(err)?;

    state.cancels.lock().unwrap().remove(&id);
    let mut recording = state.store.get(&id).map_err(err)?;
    match result {
        Ok(transcript) => {
            state.store.save_transcript(&id, &transcript).map_err(err)?;
            recording.status = Status::Done;
            recording.duration = Some(transcript.duration);
            state.store.save(&recording).map_err(err)?;
            Ok(transcript)
        }
        Err(e) => {
            let message = format!("{e:#}");
            recording.status = Status::Failed;
            recording.error = Some(message.clone());
            state.store.save(&recording).map_err(err)?;
            Err(message)
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let home = app.path().home_dir()?;
            let store = Store::new(data_dir)?;

            // 처리 도중 앱이 꺼졌던 녹음은 다시 시작할 수 있게 실패로 돌려 둔다
            for mut r in store.list()?.into_iter() {
                match r.status {
                    Status::Processing => {
                        r.status = Status::Failed;
                        r.error = Some("처리 도중 앱이 종료됐습니다".into());
                    }
                    // WAV 헤더를 2초마다 갱신하므로 마지막 2초 정도만 빠지고 나머지는 재생된다
                    Status::Recording => {
                        r.status = Status::New;
                        r.duration = finalize_tracks(&store, &r).ok();
                        r.error = Some("녹음 도중 앱이 종료됐습니다. 마지막 몇 초가 빠졌을 수 있습니다".into());
                    }
                    _ => continue,
                }
                store.save(&r)?;
            }

            app.manage(AppState {
                store,
                home,
                whisper: Mutex::new(None),
                cancels: Mutex::new(HashMap::new()),
                pipeline: tokio::sync::Mutex::new(()),
                recorder: Mutex::new(None),
                summaries: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_recordings,
            import_recordings,
            get_recording,
            update_recording,
            delete_recording,
            cancel_processing,
            process_recording,
            list_input_devices,
            start_recording,
            stop_recording,
            summary_providers,
            default_summary_prompt,
            summarize_recording,
            cancel_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
