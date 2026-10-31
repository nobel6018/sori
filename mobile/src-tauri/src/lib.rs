//! sori iPhone 앱. 녹음과 메모를 하고, 데스크톱이 만든 전사와 요약을 iCloud Drive로 받아 읽는다.
//! 받아 적기와 요약은 하지 않는다(모델이 크고 휴대폰에서 느림).
//! @see docs/mobile.md

mod intents;
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

/// 메모나 북마크(글이 빈 메모)를 더한다. time이 없으면 지금 녹음한 시간.
/// 네이티브(Swift)를 부르는 명령은 모두 async + spawn_blocking이다. 동기 명령은 메인 스레드에서 돌아,
/// 메인 스레드에서 답하는 Swift 쪽과 서로 기다리며 앱이 멈췄다(0.6.0 녹음 화면이 0:00에서 멈춤)
#[tauri::command]
async fn add_note(app: AppHandle, id: String, time: Option<f64>, text: String) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let time = match time {
            Some(t) => t.max(0.0),
            None => app.sori_native().recorder_state().map(|s| s.time).unwrap_or(0.0),
        };
        let device = sync::device_id(&state.store);
        let note = json!({ "time": time, "text": text.trim(), "id": uuid::Uuid::new_v4().to_string(), "device": device });
        let meta = state
            .store
            .update(&id, &mut |m| {
                if !m["notes"].is_array() {
                    m["notes"] = json!([]);
                }
                let notes = m["notes"].as_array_mut().unwrap();
                let at = notes.partition_point(|n| n["time"].as_f64().unwrap_or(0.0) <= time);
                notes.insert(at, note.clone());
            })
            .map_err(err)?;
        changed(&app, &id);
        Ok(meta)
    })
    .await
    .map_err(err)?
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

/// 녹음을 시작한다. 화면의 녹음 버튼과 단축어, 동작 버튼(intents.rs)이 함께 쓴다.
/// 메인 스레드에서 부르면 안 된다(add_note 설명 참고)
pub(crate) fn begin_recording(app: &AppHandle) -> CmdResult<Value> {
    let state = app.state::<AppState>();
    // 화면 버튼과 동작 버튼이 겹쳐 눌려도 하나만 시작하도록, 확인과 자리 잡기를 한 번에 한다
    let (meta, id) = {
        let mut current = state.recording.lock().unwrap();
        if current.is_some() {
            return Err("이미 녹음 중입니다".to_string());
        }
        let title = format!("녹음 {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
        let meta = state.store.create_recording(&title).map_err(err)?;
        let id = meta["id"].as_str().unwrap_or_default().to_string();
        *current = Some(id.clone());
        (meta, id)
    };
    // Live Activity를 녹음보다 먼저 띄운다. 앱 밖(동작 버튼)에서 시작할 때 iOS는 Live Activity가 떠 있어야
    // 백그라운드에서 오디오 세션을 켜 준다. 단축어는 인텐트 안에서 이미 띄웠고, 여기서는 화면 버튼일 때 띄운다.
    // 화면 버튼이면 Live Activity 없이도 녹음되므로 못 띄워도 이어 간다
    let activity = intents::activity_started();
    let started = state
        .store
        .audio_path(&meta)
        .map_err(err)
        .and_then(|path| app.sori_native().start_recording(&path.to_string_lossy()).map_err(err));
    if let Err(e) = started {
        intents::activity_stopped();
        *state.recording.lock().unwrap() = None;
        let _ = state.store.delete(&id);
        return Err(match activity {
            Ok(()) => e,
            Err(why) => format!("{e} (Live Activity를 띄우지 못했습니다: {why})"),
        });
    }
    let _ = app.emit("recorder-changed", json!({ "id": id }));
    Ok(meta)
}

/// 녹음을 멈추고 바로 동기화한다. 화면의 멈춤 버튼, Live Activity 멈춤 버튼, 동작 버튼이 함께 쓴다
pub(crate) fn end_recording(app: &AppHandle) -> CmdResult<Value> {
    let state = app.state::<AppState>();
    let id = state.recording.lock().unwrap().clone().ok_or("녹음 중이 아닙니다")?;
    let duration = app.sori_native().stop_recording().map_err(err)?;
    *state.recording.lock().unwrap() = None;
    intents::activity_stopped();
    let _ = app.emit("recorder-changed", json!({ "id": id }));
    let meta = state
        .store
        .update(&id, &mut |m| {
            m["status"] = json!("new");
            m["duration"] = json!(duration);
        })
        .map_err(err)?;
    // 녹음은 이 기기에만 있는 동안 잃을 수 있어(앱 서명 만료 등) 바로 올린다
    changed(app, &id);
    Ok(meta)
}

pub(crate) fn is_recording(app: &AppHandle) -> bool {
    app.state::<AppState>().recording.lock().unwrap().is_some()
}

#[tauri::command]
async fn start_recording(app: AppHandle) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || begin_recording(&app)).await.map_err(err)?
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || end_recording(&app)).await.map_err(err)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recorder {
    id: Option<String>,
    #[serde(flatten)]
    state: RecorderState,
}

#[tauri::command]
async fn recorder_state(app: AppHandle) -> CmdResult<Recorder> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = app.state::<AppState>().recording.lock().unwrap().clone();
        let native = if id.is_some() { app.sori_native().recorder_state().unwrap_or_default() } else { RecorderState::default() };
        Recorder { id, state: native }
    })
    .await
    .map_err(err)
}

#[tauri::command]
fn sync_status(state: State<AppState>) -> sync::SyncStatus {
    state.syncer.status()
}

#[tauri::command]
fn sync_now(app: AppHandle) {
    sync::request_now(&app);
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
async fn use_local_only(app: AppHandle) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        app.sori_native().forget_folder().map_err(err)?;
        sync::save_settings(&app.state::<AppState>().store, &sync::SyncSettings { provider: sync::Provider::Local }).map_err(err)?;
        sync::set_status(&app, |s| *s = sync::SyncStatus::default());
        Ok(())
    })
    .await
    .map_err(err)?
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
            // 녹음 도중 앱이 꺼졌으면 Live Activity만 남아 있다
            intents::activity_stopped();
            app.manage(AppState { store, recording: Mutex::new(None), syncer: sync::Syncer::new() });
            sync::start(app.handle().clone());
            // 단축어가 앱을 띄웠으면 이 시점부터 녹음을 시작할 수 있다
            intents::set_app(app.handle().clone());
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
