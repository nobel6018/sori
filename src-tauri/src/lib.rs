mod actions;
pub mod engine;
pub mod download;
pub mod models;
mod recorder;
pub mod store;
pub mod search;
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
    /// 모델 내려받기 중이면 취소 신호
    model_install: Mutex<Option<Arc<AtomicBool>>>,
    search: search::Index,
    /// 시작할 때 전역 단축키를 등록하지 못했으면 그 이유. 화면이 준비되면 알린다
    shortcut_error: Mutex<Option<String>>,
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
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<()> {
    let previous = state.store.get_settings(&state.home);
    if previous.shortcuts != settings.shortcuts {
        // 등록에 실패하면 이전 단축키로 되돌리고 저장하지 않는다
        actions::apply_shortcuts(&app, &settings.shortcuts, Some(&previous.shortcuts))?;
        state.shortcut_error.lock().unwrap().take();
    }
    state.store.save_settings(&settings).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyInfo {
    pending: Vec<actions::Action>,
    shortcut_error: Option<String>,
}

/// 화면이 app-action 이벤트를 구독한 뒤 부른다. 앱을 띄운 딥링크처럼 그 전에 들어온 요청을 넘겨준다
#[tauri::command]
fn frontend_ready(app: AppHandle, state: State<AppState>) -> ReadyInfo {
    ReadyInfo {
        pending: actions::take_pending(&app),
        shortcut_error: state.shortcut_error.lock().unwrap().clone(),
    }
}

#[tauri::command]
async fn search_recordings(app: AppHandle, query: String) -> CmdResult<Vec<search::RecordingHits>> {
    // 녹음이 많으면 파일을 읽는 데 시간이 걸리므로 화면 스레드를 막지 않는다
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        search::search(&state.store, &state.search, &query)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn model_catalog(state: State<AppState>) -> models::Catalog {
    models::catalog(&state.store.models_dir, &state.home)
}

/// 고른 Whisper 모델과 없는 화자 분리 모델을 받는다. 진행률은 model-download 이벤트로 보낸다
#[tauri::command]
async fn install_models(app: AppHandle, whisper: Vec<String>) -> CmdResult<models::Catalog> {
    let state = app.state::<AppState>();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.model_install.lock().unwrap();
        if slot.is_some() {
            return Err("이미 모델을 받는 중입니다".into());
        }
        *slot = Some(cancel.clone());
    }
    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let plan = models::install_plan(&state.store.models_dir, &state.home, &whisper).map_err(|e| eyre::eyre!(e))?;
        download::download_all(&plan, &cancel, |p| {
            let _ = worker.emit("model-download", p);
        })
    })
    .await
    .map_err(err);
    state.model_install.lock().unwrap().take();
    result?.map_err(|e| format!("{e:#}"))?;

    // 설정이 없는 파일을 가리키고 있을 때만 새로 받은 파일로 바꾼다. 사용자가 고른 경로는 두고
    let catalog = models::catalog(&state.store.models_dir, &state.home);
    let mut settings = state.store.get_settings(&state.home);
    let m = &mut settings.models;
    if !m.whisper.is_file() {
        if let Some(p) = catalog.whisper.iter().find_map(|w| w.path.clone()) {
            m.whisper = p;
        }
    }
    if !m.segmentation.is_file() {
        if let Some(p) = catalog.segmentation.clone() {
            m.segmentation = p;
        }
    }
    if !m.embedding.is_file() {
        if let Some(p) = catalog.embedding.clone() {
            m.embedding = p;
        }
    }
    state.store.save_settings(&settings).map_err(err)?;
    Ok(catalog)
}

#[tauri::command]
fn cancel_model_install(state: State<AppState>) {
    if let Some(flag) = state.model_install.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
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
    // 패닉해도 녹음 상태는 아래에서 마무리한다. 여기서 빠져나가면 "녹음 중"에 묶인다
    .unwrap_or_else(|e| Err(eyre::eyre!("트랙을 합치다 멈췄습니다: {e}")));

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
    let settings = state.store.get_settings(&state.home);
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
    // 다음 요약에서 같은 CLI를 미리 고르도록 기억한다. 요약하는 1분 사이 사용자가 바꾼 설정을
    // 덮지 않도록 저장 직전에 다시 읽고 이 값만 바꾼다
    let mut latest = state.store.get_settings(&state.home);
    latest.summary_provider = Some(provider);
    let _ = state.store.save_settings(&latest);
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
    // 모델이 없으면 whisper.cpp의 알아보기 힘든 오류 대신 설치 방법을 알려 준다
    models::check(&state.store.get_settings(&state.home).models)?;

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
    let worker_cancel = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // 앞 작업을 기다리는 동안 취소했으면 시작하지 않는다
        if worker_cancel.load(Ordering::Relaxed) {
            eyre::bail!("취소했습니다");
        }
        let state = worker_app.state::<AppState>();
        let settings = state.store.get_settings(&state.home);
        let recording = state.store.get(&worker_id)?;
        let audio = state.store.audio_path(&recording)?;
        let whisper = state.whisper(&settings.models.whisper)?;
        let track_paths = state.store.track_paths(&recording)?;
        let find = |name: &str| track_paths.iter().find(|p| p.ends_with(name)).filter(|p| p.exists()).cloned();
        let tracks = find("mic.wav").zip(find("system.wav"));

        let emitter = worker_app.clone();
        let event_id = worker_id.clone();
        engine::process(
            &audio,
            &whisper,
            &settings.models,
            &ProcessOptions {
                num_speakers: request.num_speakers,
                languages: request.languages,
                tracks,
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
    // 패닉해도 상태를 "실패"로 남기고 취소 목록을 비운다. 여기서 빠져나가면 "처리 중"에 묶여 삭제도 안 된다
    .unwrap_or_else(|e| Err(eyre::eyre!("처리하다 멈췄습니다: {e}")));

    state.cancels.lock().unwrap().remove(&id);
    let mut recording = state.store.get(&id).map_err(err)?;
    match result {
        Ok(transcript) => {
            state.store.save_transcript(&id, &transcript).map_err(err)?;
            // 다시 처리하면 화자 번호가 바뀔 수 있어, 예전 번호에 붙인 이름은 지운다
            recording.speaker_names.clear();
            // 마이크 쪽 화자가 한 명뿐이면 녹음한 본인이다
            let locals: Vec<i32> = transcript
                .origins
                .iter()
                .filter(|(_, o)| **o == engine::origin::Origin::Local)
                .map(|(s, _)| *s)
                .collect();
            if let [me] = locals.as_slice() {
                recording.speaker_names.insert(me.to_string(), "나".to_string());
            }
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
        // 다른 플러그인보다 먼저 등록해야 두 번째 실행을 가장 먼저 가로챈다
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // 딥링크는 deep-link 플러그인이 넘겨받는다. 녹음 토글 링크로 창이 앞에 뜨지 않도록 여기서는 건드리지 않는다
            if args.iter().any(|a| a.starts_with("sori:")) {
                return;
            }
            let files = opened_files(args.iter().skip(1).map(PathBuf::from));
            if files.is_empty() {
                actions::show_main(app);
            } else {
                actions::dispatch(app, actions::Action::Import { paths: files, process: false });
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        actions::on_shortcut(app, shortcut);
                    }
                })
                .build(),
        )
        .manage(actions::Bus::new())
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
                model_install: Mutex::new(None),
                search: search::Index::default(),
                shortcut_error: Mutex::new(None),
            });

            let handle = app.handle().clone();
            let settings = app.state::<AppState>().store.get_settings(&home_dir(&handle));
            if let Err(e) = actions::apply_shortcuts(&handle, &settings.shortcuts, None) {
                *app.state::<AppState>().shortcut_error.lock().unwrap() = Some(e);
            }

            setup_deep_links(app)?;
            // Windows는 파일을 sori로 열면 경로를 실행 인자로 넘긴다. macOS는 RunEvent::Opened로 온다
            let files = opened_files(std::env::args_os().skip(1).map(PathBuf::from));
            if !files.is_empty() {
                actions::dispatch(&handle, actions::Action::Import { paths: files, process: false });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            model_catalog,
            install_models,
            cancel_model_install,
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
            frontend_ready,
            search_recordings,
            reveal_audio,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Finder에서 "다음으로 열기 > sori"를 고르거나 Dock 아이콘에 파일을 끌어다 놓은 경우
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = event {
                let files = opened_files(urls.iter().filter_map(|u| u.to_file_path().ok()));
                if !files.is_empty() {
                    actions::dispatch(app, actions::Action::Import { paths: files, process: false });
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

fn home_dir(app: &AppHandle) -> PathBuf {
    app.path().home_dir().unwrap_or_default()
}

/// 실행 인자나 열기 요청 중 실제로 있는 파일만 고른다. 오디오인지는 가져올 때 화면이 거른다
fn opened_files(paths: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths.filter(|p| p.is_file()).collect()
}

fn setup_deep_links(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_deep_link::DeepLinkExt;
    // 설치 파일이 스킴을 등록한다. 개발 중(cargo run)인 Windows와 Linux만 직접 등록한다
    #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
    app.deep_link().register_all()?;

    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            actions::handle_url(&handle, &url);
        }
    });
    // 링크로 앱을 처음 띄운 경우. 화면이 준비되면 frontend_ready로 넘긴다
    if let Some(urls) = app.deep_link().get_current()? {
        for url in urls {
            actions::handle_url(app.handle(), &url);
        }
    }
    Ok(())
}

/// 녹음의 오디오 파일을 Finder(Windows는 탐색기)에서 보여 준다
#[tauri::command]
fn reveal_audio(state: State<AppState>, id: String) -> CmdResult<()> {
    let recording = state.store.get(&id).map_err(err)?;
    let path = state.store.audio_path(&recording).map_err(err)?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}
