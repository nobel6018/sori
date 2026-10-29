//! iPhone 동기화 연결. 규칙은 crates/sori-sync에 있고, 여기는 iCloud Drive 폴더를 여는 일과 주기 실행만 한다.
//! @see docs/sync.md, docs/mobile.md

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sori_sync::{AudioUpload, FolderRemote, Local};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_sori_native::SoriNativeExt;

use crate::store::Store;
use crate::AppState;

/// 앱이 앞에 있는 동안 다른 기기의 변경을 확인하는 간격. 휴대폰은 앱을 열 때와 녹음을 멈출 때도 맞춘다
const INTERVAL: Duration = Duration::from_secs(60);
const DEBOUNCE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Icloud,
    /// 이 기기에만 저장. 모르는 값도 여기로 읽는다
    #[default]
    #[serde(other)]
    Local,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    pub provider: Provider,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub provider: Provider,
    /// 고른 iCloud Drive 폴더 이름
    pub folder: Option<String>,
    pub running: bool,
    pub last_sync: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub message: Option<String>,
}

pub struct Syncer {
    trigger: Mutex<Option<mpsc::Sender<()>>>,
    status: Mutex<SyncStatus>,
}

impl Syncer {
    pub fn new() -> Self {
        Self { trigger: Mutex::new(None), status: Mutex::new(SyncStatus::default()) }
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }
}

pub fn load_settings(store: &Store) -> SyncSettings {
    std::fs::read(store.data_file("sync.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_settings(store: &Store, settings: &SyncSettings) -> Result<()> {
    std::fs::write(store.data_file("sync.json"), serde_json::to_vec_pretty(settings)?)?;
    Ok(())
}

/// iCloud Drive 루트나 상위 폴더를 골랐으면 그 안의 sori 폴더(Mac 앱이 만든 곳)를 쓴다
pub fn sori_folder(picked: &Path) -> PathBuf {
    let inner = picked.join("sori");
    if !picked.join("recordings").is_dir() && inner.is_dir() { inner } else { picked.to_path_buf() }
}

pub fn request(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().syncer.trigger.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<()>();
    *app.state::<AppState>().syncer.trigger.lock().unwrap() = Some(tx);
    std::thread::spawn(move || {
        let mut first = true;
        loop {
            if !first {
                match rx.recv_timeout(INTERVAL) {
                    Ok(()) => {
                        std::thread::sleep(DEBOUNCE);
                        while rx.try_recv().is_ok() {}
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            first = false;
            run_once(&app);
        }
    });
}

pub fn set_status(app: &AppHandle, change: impl FnOnce(&mut SyncStatus)) {
    let state = app.state::<AppState>();
    let status = {
        let mut s = state.syncer.status.lock().unwrap();
        change(&mut s);
        s.clone()
    };
    let _ = app.emit("sync-status", status);
}

/// 설정이 iCloud면 저장해 둔 폴더 권한을 열어 클라우드를 만든다
fn remote(app: &AppHandle) -> Result<Option<(FolderRemote, String)>> {
    let state = app.state::<AppState>();
    if load_settings(&state.store).provider != Provider::Icloud {
        return Ok(None);
    }
    let folder = app
        .sori_native()
        .resolve_folder()
        .map_err(|e| eyre!("{e}"))?
        .ok_or_else(|| eyre!("동기화 폴더를 고르지 않았습니다. 설정에서 iCloud Drive의 sori 폴더를 골라 주세요"))?;
    let root = sori_folder(Path::new(&folder.path));
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    set_status(app, |s| s.folder = Some(name));
    let native = app.clone();
    let remote = FolderRemote::new(root.clone()).with_materialize(Box::new(move |p: &Path| {
        native.sori_native().materialize(&p.to_string_lossy()).map_err(|e| eyre!("{e}"))
    }));
    Ok(Some((remote, format!("iCloud Drive:{}", root.display()))))
}

fn run_once(app: &AppHandle) {
    let state = app.state::<AppState>();
    let provider = load_settings(&state.store).provider;
    let (remote, key) = match remote(app) {
        Ok(Some(r)) => r,
        Ok(None) => {
            set_status(app, |s| *s = SyncStatus { provider, ..Default::default() });
            return;
        }
        Err(e) => {
            set_status(app, |s| {
                s.provider = provider;
                s.running = false;
                s.error = Some(format!("{e:#}"));
            });
            return;
        }
    };
    set_status(app, |s| {
        s.provider = provider;
        s.running = true;
        s.message = None;
    });
    let local = MobileLocal::new(app);
    let progress_app = app.clone();
    let progress = move |done: u64, total: u64| {
        if done % (1 << 20) < (1 << 16) || done == total {
            set_status(&progress_app, |s| s.message = Some(format!("오디오 올리는 중 {} / {}MB", done >> 20, (total >> 20).max(1))));
        }
    };
    let result = sori_sync::sync(&local, &remote, &key, &state.store.data_file("sync-state.json"), &progress);
    match result {
        Ok(report) => {
            set_status(app, |s| {
                s.running = false;
                s.message = None;
                s.last_sync = Some(Utc::now());
                s.error = (!report.errors.is_empty()).then(|| report.errors.join("\n"));
            });
            if report.downloaded + report.deleted_local > 0 {
                let _ = app.emit("recordings-synced", ());
            }
        }
        Err(e) => set_status(app, |s| {
            s.running = false;
            s.message = None;
            s.error = Some(format!("{e:#}"));
        }),
    }
}

/// 클라우드에만 있는 오디오를 받는다
pub fn fetch_audio(app: &AppHandle, id: &str) -> Result<()> {
    let (remote, _) = remote(app)?.ok_or_else(|| eyre!("동기화가 꺼져 있어 오디오를 받을 수 없습니다"))?;
    let local = MobileLocal::new(app);
    let event_app = app.clone();
    let event_id = id.to_string();
    let progress = move |done: u64, total: u64| {
        let _ = event_app.emit("audio-download", json!({ "id": event_id, "done": done, "total": total }));
    };
    sori_sync::download_audio(&local, &remote, id, &progress)?;
    Ok(())
}

pub fn device_id(store: &Store) -> String {
    let path = store.data_file("device-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        if !id.trim().is_empty() {
            return id.trim().to_string();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::write(&path, &id);
    id
}

struct MobileLocal<'a> {
    app: &'a AppHandle,
    store: &'a Store,
    device: String,
}

impl<'a> MobileLocal<'a> {
    fn new(app: &'a AppHandle) -> Self {
        let store = &app.state::<AppState>().inner().store;
        Self { app, store, device: device_id(store) }
    }
}

impl Local for MobileLocal<'_> {
    fn device_id(&self) -> String {
        self.device.clone()
    }

    fn recording_ids(&self) -> Result<Vec<String>> {
        self.store.ids()
    }

    fn recording_dir(&self, id: &str) -> PathBuf {
        self.store.dir(id).unwrap_or_default()
    }

    /// 지금 녹음 중인 항목만 건너뛴다. 휴대폰은 처리(받아 적기)를 하지 않는다
    fn is_busy(&self, id: &str) -> bool {
        self.app.state::<AppState>().recording.lock().unwrap().as_deref() == Some(id)
    }

    fn read_meta(&self, id: &str) -> Result<Value> {
        self.store.read(id)
    }

    fn update_meta(&self, id: &str, change: &mut dyn FnMut(&mut Value)) -> Result<Value> {
        let meta = self.store.update(id, change)?;
        let _ = self.app.emit("recording-changed", json!({ "id": id }));
        Ok(meta)
    }

    fn create_recording(&self, id: &str, meta: Value) -> Result<()> {
        self.store.create_raw(id, meta)
    }

    fn delete_recording(&self, id: &str) -> Result<()> {
        self.store.delete(id)
    }

    /// 휴대폰 녹음은 처음부터 AAC m4a라 그대로 올린다
    fn prepare_audio(&self, _: &str, source: &Path) -> Result<AudioUpload> {
        let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "m4a".into());
        Ok(AudioUpload { path: source.to_path_buf(), ext, temporary: false })
    }

    fn transcript_arrived(&self, id: &str) -> Result<()> {
        self.store.update(id, &mut |m| {
            if matches!(m["status"].as_str(), Some("new") | Some("failed") | Some("processing")) {
                m["status"] = json!("done");
            }
        })?;
        Ok(())
    }
}
