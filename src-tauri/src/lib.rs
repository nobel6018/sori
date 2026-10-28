pub mod engine;
mod store;

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
            for mut r in store.list()?.into_iter().filter(|r| r.status == Status::Processing) {
                r.status = Status::Failed;
                r.error = Some("처리 도중 앱이 종료됐습니다".into());
                store.save(&r)?;
            }

            app.manage(AppState {
                store,
                home,
                whisper: Mutex::new(None),
                cancels: Mutex::new(HashMap::new()),
                pipeline: tokio::sync::Mutex::new(()),
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
