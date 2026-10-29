//! sori iPhone 앱. 녹음과 메모를 하고, 데스크톱이 만든 전사와 요약을 iCloud Drive로 받아 읽는다.
//! 받아 적기와 요약은 하지 않는다(모델이 크고 휴대폰에서 느림).
//! @see docs/mobile.md

mod store;
mod sync;

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_sori_native::{RecorderState, SoriNativeExt};

use store::Store;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) struct AppState {
    store: Store,
    /// 지금 녹음 중인 녹음 id
    recording: Mutex<Option<String>>,
    syncer: sync::Syncer,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingDetail {
    meta: Value,
    transcript: Option<Value>,
    summaries: Vec<Value>,
    audio_path: PathBuf,
    audio_available: bool,
}

/// 목록 한 줄. meta에 전사와 오디오가 이 기기에 있는지를 더한다
fn summary_of(store: &Store, mut meta: Value) -> Value {
    let id = meta["id"].as_str().unwrap_or_default().to_string();
    let has_transcript = store.dir(&id).map(|d| d.join("transcript.json").is_file()).unwrap_or(false);
    let audio = store.audio_path(&meta).map(|p| p.is_file()).unwrap_or(false);
    meta["hasTranscript"] = json!(has_transcript);
    meta["audioAvailable"] = json!(audio);
    meta
}

fn changed(app: &AppHandle, id: &str) {
    let _ = app.emit("recording-changed", json!({ "id": id }));
    sync::request(app);
}

#[tauri::command]
fn list_recordings(state: State<AppState>) -> CmdResult<Vec<Value>> {
    Ok(state.store.list().map_err(err)?.into_iter().map(|m| summary_of(&state.store, m)).collect())
}

#[tauri::command]
fn get_recording(state: State<AppState>, id: String) -> CmdResult<RecordingDetail> {
    let meta = state.store.read(&id).map_err(err)?;
    let audio_path = state.store.audio_path(&meta).map_err(err)?;
    Ok(RecordingDetail {
        transcript: state.store.transcript(&id).map_err(err)?,
        summaries: state.store.summaries(&id).map_err(err)?,
        audio_available: audio_path.is_file(),
        audio_path,
        meta,
    })
}

#[tauri::command]
fn rename_recording(app: AppHandle, state: State<AppState>, id: String, title: String) -> CmdResult<Value> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("제목이 비어 있습니다".into());
    }
    let meta = state
        .store
        .update(&id, &mut |m| {
            m["title"] = json!(title);
            // 다른 기기와 동시에 고쳤을 때 늦게 고친 쪽을 고르는 기준(docs/sync.md)
            m["updatedAt"] = json!(Utc::now().to_rfc3339());
        })
        .map_err(err)?;
    changed(&app, &id);
    Ok(meta)
}

#[tauri::command]
fn delete_recording(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<()> {
    if state.recording.lock().unwrap().as_deref() == Some(id.as_str()) {
        return Err("녹음 중인 항목은 지울 수 없습니다".into());
    }
    state.store.delete(&id).map_err(err)?;
    sync::request(&app);
    Ok(())
}

/// 메모나 북마크(글이 빈 메모)를 더한다. time이 없으면 지금 녹음한 시간
#[tauri::command]
fn add_note(app: AppHandle, state: State<AppState>, id: String, time: Option<f64>, text: String) -> CmdResult<Value> {
    let time = match time {
        Some(t) => t.max(0.0),
        None => app.sori_native().recorder_state().map(|s| s.time).unwrap_or(0.0),
    };
    let device = sync::device_id(&state.store);
    let note = json!({ "time": time, "text": text.trim(), "id": uuid::Uuid::new_v4().to_string(), "device": device });
    let meta = state
        .store
        .update(&id, &mut |m| {
            let notes = m["notes"].as_array_mut();
            let notes = match notes {
                Some(n) => n,
                None => {
                    m["notes"] = json!([]);
                    m["notes"].as_array_mut().unwrap()
                }
            };
            let at = notes.partition_point(|n| n["time"].as_f64().unwrap_or(0.0) <= time);
            notes.insert(at, note.clone());
        })
        .map_err(err)?;
    changed(&app, &id);
    Ok(meta)
}

#[tauri::command]
fn delete_note(app: AppHandle, state: State<AppState>, id: String, note_id: String) -> CmdResult<Value> {
    let meta = state
        .store
        .update(&id, &mut |m| {
            if let Some(notes) = m["notes"].as_array_mut() {
                notes.retain(|n| n["id"].as_str() != Some(note_id.as_str()));
            }
        })
        .map_err(err)?;
    changed(&app, &id);
    Ok(meta)
}

#[tauri::command]
async fn start_recording(app: AppHandle) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.recording.lock().unwrap().is_some() {
            return Err("이미 녹음 중입니다".to_string());
        }
        let title = format!("녹음 {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
        let meta = state.store.create_recording(&title).map_err(err)?;
        let id = meta["id"].as_str().unwrap_or_default().to_string();
        let path = state.store.audio_path(&meta).map_err(err)?;
        if let Err(e) = app.sori_native().start_recording(&path.to_string_lossy()) {
            let _ = state.store.delete(&id);
            return Err(e.to_string());
        }
        *state.recording.lock().unwrap() = Some(id);
        Ok(meta)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let id = state.recording.lock().unwrap().clone().ok_or("녹음 중이 아닙니다")?;
        let duration = app.sori_native().stop_recording().map_err(err)?;
        *state.recording.lock().unwrap() = None;
        let meta = state
            .store
            .update(&id, &mut |m| {
                m["status"] = json!("new");
                m["duration"] = json!(duration);
            })
            .map_err(err)?;
        // 녹음은 이 기기에만 있는 동안 잃을 수 있어(앱 서명 만료 등) 바로 올린다
        changed(&app, &id);
        Ok(meta)
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recorder {
    id: Option<String>,
    #[serde(flatten)]
    state: RecorderState,
}

#[tauri::command]
fn recorder_state(app: AppHandle, state: State<AppState>) -> Recorder {
    let id = state.recording.lock().unwrap().clone();
    let native = if id.is_some() { app.sori_native().recorder_state().unwrap_or_default() } else { RecorderState::default() };
    Recorder { id, state: native }
}

#[tauri::command]
fn sync_status(state: State<AppState>) -> sync::SyncStatus {
    state.syncer.status()
}

#[tauri::command]
fn sync_now(app: AppHandle) {
    sync::request(&app);
}

/// iCloud Drive에서 동기화 폴더를 고른다. 취소하면 false
#[tauri::command]
async fn choose_icloud_folder(app: AppHandle) -> CmdResult<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(_) = app.sori_native().pick_folder().map_err(err)? else { return Ok(false) };
        let state = app.state::<AppState>();
        sync::save_settings(&state.store, &sync::SyncSettings { provider: sync::Provider::Icloud }).map_err(err)?;
        sync::request(&app);
        Ok(true)
    })
    .await
    .map_err(err)?
}

/// 동기화를 끄고 이 기기에만 저장한다. 이미 받은 녹음은 남는다
#[tauri::command]
fn use_local_only(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    app.sori_native().forget_folder().map_err(err)?;
    sync::save_settings(&state.store, &sync::SyncSettings { provider: sync::Provider::Local }).map_err(err)?;
    sync::set_status(&app, |s| *s = sync::SyncStatus::default());
    Ok(())
}

#[tauri::command]
async fn download_audio(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || sync::fetch_audio(&app, &id).map_err(|e| format!("{e:#}")))
        .await
        .map_err(err)?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sori_native::init())
        .setup(|app| {
            let store = Store::new(app.path().app_data_dir()?)?;
            // 녹음 도중 앱이 꺼졌으면 녹음된 데까지 남아 있다. 이어서 쓸 수 있게 상태를 돌려 둔다
            for meta in store.list()? {
                if meta["status"] == "recording" {
                    if let Some(id) = meta["id"].as_str() {
                        store.update(id, &mut |m| m["status"] = json!("new"))?;
                    }
                }
            }
            app.manage(AppState { store, recording: Mutex::new(None), syncer: sync::Syncer::new() });
            sync::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_recordings,
            get_recording,
            rename_recording,
            delete_recording,
            add_note,
            delete_note,
            start_recording,
            stop_recording,
            recorder_state,
            sync_status,
            sync_now,
            choose_icloud_folder,
            use_local_only,
            download_audio,
        ])
        .run(tauri::generate_context!())
        .expect("error while running sori");
}
