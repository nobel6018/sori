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
const INTERVAL: Duration = Duration::from_secs(20);
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
    /// true면 사용자가 누른 "지금 동기화"라 모으지 않고 바로 맞춘다
    trigger: Mutex<Option<mpsc::Sender<bool>>>,
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

/// Files에서 고른 폴더로 sori 루트(Mac 앱이 만든 iCloud Drive/sori)를 찾는다.
/// sori 안의 recordings를 고르는 실수가 잦아, 고른 폴더를 그대로 쓰던 때는 sori/recordings/recordings가 생겼다.
/// - 고른 폴더나 그 위에 sori 폴더가 있으면 그것(sori/recordings, sori/recordings/<id>를 골라도 sori)
/// - iCloud Drive처럼 위 폴더를 골랐으면 그 안의 sori
/// - 이름을 바꾼 루트라도 안에 recordings가 있으면 그 폴더
/// - 어느 것도 아니면 거절한다. 상관없는 폴더에 동기화 파일을 만들지 않는다
pub fn sori_root(picked: &Path) -> Result<PathBuf> {
    if let Some(root) = picked.ancestors().find(|p| p.file_name().is_some_and(|n| n == "sori")) {
        return Ok(root.to_path_buf());
    }
    let inner = picked.join("sori");
    if inner.is_dir() {
        return Ok(inner);
    }
    if picked.join("recordings").is_dir() && picked.file_name().is_some_and(|n| n != "recordings") {
        return Ok(picked.to_path_buf());
    }
    let name = picked.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Err(eyre!(
        "'{name}'은 sori 폴더가 아닙니다. iCloud Drive 안의 sori 폴더를 골라 주세요. Mac의 sori 설정에서 동기화를 iCloud Drive로 켜면 생깁니다"
    ))
}

/// 예전에 sori/recordings를 루트로 골라 생긴 sori/recordings/recordings/<id>, sori/recordings/deleted/<id>.json을
/// 제자리(sori/recordings/<id>, sori/deleted/<id>.json)로 옮긴다. 같은 이름이 이미 있으면 덮어쓰지 않고 남겨 둔다
pub fn merge_nested(root: &Path) -> Result<usize> {
    let mut moved = 0;
    for (nested, dest) in [(root.join("recordings/recordings"), root.join("recordings")), (root.join("recordings/deleted"), root.join("deleted"))] {
        let Ok(entries) = std::fs::read_dir(&nested) else { continue };
        std::fs::create_dir_all(&dest)?;
        for entry in entries.flatten() {
            let to = dest.join(entry.file_name());
            if to.exists() {
                continue;
            }
            std::fs::rename(entry.path(), &to)?;
            moved += 1;
        }
        // 다 옮겨 비었을 때만 지워진다
        let _ = std::fs::remove_dir(&nested);
    }
    Ok(moved)
}

pub fn request(app: &AppHandle) {
    send(app, false);
}

/// 사용자가 누른 동기화. 변경을 모으는 대기 없이 바로 맞춘다
pub fn request_now(app: &AppHandle) {
    send(app, true);
}

fn send(app: &AppHandle, now: bool) {
    if let Some(tx) = app.state::<AppState>().syncer.trigger.lock().unwrap().as_ref() {
        let _ = tx.send(now);
    }
}

pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<bool>();
    *app.state::<AppState>().syncer.trigger.lock().unwrap() = Some(tx);
    std::thread::spawn(move || {
        let mut first = true;
        loop {
            if !first {
                match rx.recv_timeout(INTERVAL) {
                    Ok(now) => {
                        // 메모를 여러 개 남기는 등 변경이 몰리면 잠깐 모았다 한 번에 맞춘다. 그 사이 "지금"이 오면 바로 간다
                        if !now {
                            let deadline = std::time::Instant::now() + DEBOUNCE;
                            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                                match rx.recv_timeout(left) {
                                    Ok(true) | Err(mpsc::RecvTimeoutError::Timeout) => break,
                                    Ok(false) => continue,
                                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                                }
                            }
                        }
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
    let root = sori_root(Path::new(&folder.path))?;
    match merge_nested(&root) {
        Ok(0) => {}
        Ok(n) => eprintln!("sori: 잘못 고른 폴더에 생긴 녹음 {n}개를 제자리로 옮겼습니다"),
        Err(e) => eprintln!("sori: 중첩 폴더를 정리하지 못했습니다: {e:#}"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sori-sync-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sori_안의_recordings나_녹음_폴더를_골라도_sori를_쓴다() {
        let drive = tmp();
        let sori = drive.join("sori");
        std::fs::create_dir_all(sori.join("recordings/abc")).unwrap();

        assert_eq!(sori_root(&sori).unwrap(), sori);
        assert_eq!(sori_root(&sori.join("recordings")).unwrap(), sori);
        assert_eq!(sori_root(&sori.join("recordings/abc")).unwrap(), sori);
    }

    #[test]
    fn icloud_drive를_고르면_안의_sori를_쓴다() {
        let drive = tmp();
        std::fs::create_dir_all(drive.join("sori")).unwrap();

        assert_eq!(sori_root(&drive).unwrap(), drive.join("sori"));
    }

    #[test]
    fn 상관없는_폴더는_거절한다() {
        let drive = tmp();
        std::fs::create_dir_all(drive.join("Documents")).unwrap();

        assert!(sori_root(&drive.join("Documents")).is_err());
    }

    #[test]
    fn 중첩된_녹음과_지운_표시를_제자리로_옮기고_있는_것은_두다() {
        let sori = tmp().join("sori");
        std::fs::create_dir_all(sori.join("recordings/recordings/new/notes")).unwrap();
        std::fs::write(sori.join("recordings/recordings/new/meta.json"), "{}").unwrap();
        std::fs::create_dir_all(sori.join("recordings/recordings/both")).unwrap();
        std::fs::create_dir_all(sori.join("recordings/both")).unwrap();
        std::fs::create_dir_all(sori.join("recordings/deleted")).unwrap();
        std::fs::write(sori.join("recordings/deleted/gone.json"), "{}").unwrap();

        let moved = merge_nested(&sori).unwrap();

        assert_eq!(moved, 2);
        assert!(sori.join("recordings/new/meta.json").is_file());
        assert!(sori.join("deleted/gone.json").is_file());
        assert!(!sori.join("recordings/deleted").exists());
        // 같은 id가 이미 있어 옮기지 않은 것은 중첩 폴더에 남는다
        assert!(sori.join("recordings/recordings/both").is_dir());
    }
}
