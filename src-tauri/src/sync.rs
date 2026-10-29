//! 데스크톱 동기화 연결: 설정, 주기 실행, 오디오 압축, 화면 알림.
//! 동기화 규칙은 crates/sori-sync에 있다. @see docs/sync.md

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sori_sync::{AudioUpload, FolderRemote, Local, Remote};
use tauri::{AppHandle, Emitter, Manager};

use crate::store::{Recording, Status, Store};
use crate::AppState;

/// 아무 변화가 없어도 이 간격으로 다른 기기의 변경을 확인한다
const INTERVAL: Duration = Duration::from_secs(120);
/// 변경 요청이 몰려 오면(메모 여러 개 등) 잠깐 기다렸다 한 번에 맞춘다
const DEBOUNCE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    /// 이 기기에만 저장
    #[default]
    Local,
    Icloud,
    Onedrive,
    GoogleDrive,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    #[serde(default)]
    pub provider: Provider,
    /// iCloud Drive 안의 sori 폴더. None이면 기본 위치
    #[serde(default)]
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub provider: Provider,
    pub running: bool,
    pub last_sync: Option<DateTime<Utc>>,
    /// 마지막 동기화가 실패했거나 일부 녹음을 맞추지 못한 이유
    pub error: Option<String>,
    /// 지금 하는 일(오디오 올리는 중 등)
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

/// 기본 iCloud Drive 폴더. macOS는 ~/Library/Mobile Documents, Windows는 iCloud for Windows의 기본 위치
pub fn default_icloud_folder(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Mobile Documents/com~apple~CloudDocs/sori")
    } else {
        home.join("iCloudDrive").join("sori")
    }
}

/// 설정에 맞는 클라우드와 그 구분값. 로컬 전용이면 None
pub fn remote_for(settings: &SyncSettings, home: &Path) -> Result<Option<(Box<dyn Remote>, String)>> {
    match settings.provider {
        Provider::Local => Ok(None),
        Provider::Icloud => {
            let folder = settings.folder.clone().unwrap_or_else(|| default_icloud_folder(home));
            let drive = folder.parent().ok_or_else(|| eyre!("iCloud 폴더 경로가 잘못됐습니다"))?;
            if !drive.is_dir() {
                return Err(eyre!(
                    "iCloud Drive 폴더가 없습니다: {}. {}",
                    drive.display(),
                    if cfg!(target_os = "macos") {
                        "시스템 설정 > Apple 계정 > iCloud에서 iCloud Drive를 켜 주세요"
                    } else {
                        "iCloud for Windows를 설치하고 iCloud Drive를 켜 주세요"
                    }
                ));
            }
            std::fs::create_dir_all(&folder).wrap_err_with(|| format!("폴더를 만들지 못했습니다: {}", folder.display()))?;
            let key = format!("icloud:{}", folder.display());
            Ok(Some((Box::new(FolderRemote::new(folder)), key)))
        }
        Provider::Onedrive | Provider::GoogleDrive => Err(eyre!("OneDrive와 Google Drive 연결은 아직 준비 중입니다")),
    }
}

/// 동기화를 곧 한 번 하게 한다. 로컬 전용이면 아무것도 하지 않는다
pub fn request(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().syncer.trigger.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// 앱이 켜져 있는 동안 동기화를 돌리는 스레드
pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<()>();
    *app.state::<AppState>().syncer.trigger.lock().unwrap() = Some(tx);
    std::thread::spawn(move || {
        // 켜자마자 한 번 맞춘다
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

fn set_status(app: &AppHandle, change: impl FnOnce(&mut SyncStatus)) {
    let state = app.state::<AppState>();
    let status = {
        let mut s = state.syncer.status.lock().unwrap();
        change(&mut s);
        s.clone()
    };
    let _ = app.emit("sync-status", status);
}

fn run_once(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.store.get_settings(&state.home);
    let provider = settings.sync.provider;
    let remote = match remote_for(&settings.sync, &state.home) {
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
    let local = DesktopLocal::new(app);
    let progress_app = app.clone();
    let progress = move |done: u64, total: u64| {
        // 큰 오디오를 올릴 때만 의미가 있다. 1MB 단위로만 알린다
        if done % (1 << 20) < (1 << 16) || done == total {
            let message = if total > 0 {
                format!("오디오 올리는 중 {} / {}MB", done >> 20, (total >> 20).max(1))
            } else {
                format!("오디오 올리는 중 {}MB", done >> 20)
            };
            set_status(&progress_app, |s| s.message = Some(message));
        }
    };
    let (remote, key) = remote;
    let result = sori_sync::sync(&local, remote.as_ref(), &key, &state.store.data_file("sync-state.json"), &progress);
    match result {
        Ok(report) => {
            let changed = report.downloaded + report.deleted_local > 0;
            set_status(app, |s| {
                s.running = false;
                s.message = None;
                s.last_sync = Some(Utc::now());
                s.error = (!report.errors.is_empty()).then(|| report.errors.join("\n"));
            });
            if changed {
                let _ = app.emit("recordings-synced", ());
            }
            process_new_arrivals(app);
        }
        Err(e) => set_status(app, |s| {
            s.running = false;
            s.message = None;
            s.error = Some(format!("{e:#}"));
        }),
    }
}

/// 다른 기기(휴대폰 등)에서 온, 아직 받아 적지 않은 녹음을 자동 처리 설정에 따라 처리한다.
/// 오디오는 처리할 때 받는다(process_recording)
fn process_new_arrivals(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.store.get_settings(&state.home);
    if !settings.auto_process || crate::models::check(&settings.models).is_err() {
        return;
    }
    let Ok(list) = state.store.list() else { return };
    for r in list {
        let from_elsewhere = r.status == Status::New && r.error.is_none() && !state.store.audio_path(&r).is_ok_and(|p| p.is_file());
        if !from_elsewhere || state.cancels.lock().unwrap().contains_key(&r.id) {
            continue;
        }
        let app = app.clone();
        let request = crate::ProcessRequest {
            id: r.id.clone(),
            num_speakers: None,
            languages: settings.default_languages.clone(),
            diarize: None,
        };
        tauri::async_runtime::spawn(async move {
            let _ = crate::run_processing(app, request).await;
        });
    }
}

/// 클라우드에서 이 녹음의 오디오를 받는다. 받은 경로를 돌려준다
pub fn fetch_audio(app: &AppHandle, id: &str) -> Result<PathBuf> {
    let state = app.state::<AppState>();
    let settings = state.store.get_settings(&state.home);
    let (remote, _) = remote_for(&settings.sync, &state.home)?.ok_or_else(|| eyre!("오디오 파일이 없습니다. 동기화가 꺼져 있어 받아 올 수도 없습니다"))?;
    let local = DesktopLocal::new(app);
    let event_app = app.clone();
    let event_id = id.to_string();
    let progress = move |done: u64, total: u64| {
        let _ = event_app.emit("audio-download", AudioDownload { id: event_id.clone(), done, total });
    };
    let path = sori_sync::download_audio(&local, remote.as_ref(), id, &progress)?;
    let _ = app.emit("recording-changed", crate::RecordingChanged { id: id.to_string() });
    Ok(path)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioDownload {
    id: String,
    done: u64,
    total: u64,
}

/// 이 기기의 고정 id. 처음 부르면 만든다
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

/// 데스크톱 저장소(Store)를 동기화 엔진에 잇는다. app이 없으면(테스트) 화면 알림과 처리 중 확인을 건너뛴다
struct DesktopLocal<'a> {
    store: &'a Store,
    app: Option<&'a AppHandle>,
    device: String,
}

impl<'a> DesktopLocal<'a> {
    fn new(app: &'a AppHandle) -> Self {
        let store = &app.state::<AppState>().inner().store;
        Self { store, app: Some(app), device: device_id(store) }
    }

    fn store(&self) -> &Store {
        self.store
    }
}

impl Local for DesktopLocal<'_> {
    fn device_id(&self) -> String {
        self.device.clone()
    }

    fn recording_ids(&self) -> Result<Vec<String>> {
        self.store().ids()
    }

    fn recording_dir(&self, id: &str) -> PathBuf {
        self.store().recording_dir(id).unwrap_or_default()
    }

    fn is_busy(&self, id: &str) -> bool {
        if self.app.is_some_and(|app| app.state::<AppState>().cancels.lock().unwrap().contains_key(id)) {
            return true;
        }
        self.store.get(id).map_or(true, |r| matches!(r.status, Status::Recording | Status::Processing))
    }

    fn read_meta(&self, id: &str) -> Result<Value> {
        self.store().read_raw(id)
    }

    fn update_meta(&self, id: &str, change: &mut dyn FnMut(&mut Value)) -> Result<Value> {
        let value = self.store().update_raw(id, change)?;
        if let Some(app) = self.app {
            let _ = app.emit("recording-changed", crate::RecordingChanged { id: id.to_string() });
        }
        Ok(value)
    }

    fn create_recording(&self, id: &str, meta: Value) -> Result<()> {
        self.store().create_raw(id, meta)
    }

    fn delete_recording(&self, id: &str) -> Result<()> {
        self.store().delete(id)
    }

    fn prepare_audio(&self, id: &str, source: &Path) -> Result<AudioUpload> {
        let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if COMPRESSED.contains(&ext.as_str()) {
            return Ok(AudioUpload { path: source.to_path_buf(), ext, temporary: false });
        }
        if let Some(app) = self.app {
            set_status(app, |s| s.message = Some("오디오 압축하는 중".into()));
        }
        let tmp = std::env::temp_dir().join(format!("sori-upload-{id}-{}", uuid::Uuid::new_v4().simple()));
        compress(source, &tmp)
    }

    fn transcript_arrived(&self, id: &str) -> Result<()> {
        self.store().update(id, |r: &mut Recording| {
            if matches!(r.status, Status::New | Status::Failed) {
                r.status = Status::Done;
                r.error = None;
            }
        })?;
        Ok(())
    }
}

/// 다시 압축하지 않고 그대로 올리는 형식
const COMPRESSED: &[&str] = &["m4a", "mp4", "aac", "mp3", "ogg", "oga", "opus", "flac", "webm", "mka", "mkv"];

/// macOS: 운영체제의 afconvert로 AAC(m4a, 모노 48kbps). 1시간에 약 22MB
#[cfg(target_os = "macos")]
fn compress(source: &Path, tmp: &Path) -> Result<AudioUpload> {
    let dest = tmp.with_extension("m4a");
    let output = std::process::Command::new("/usr/bin/afconvert")
        .args(["-f", "m4af", "-d", "aac", "-b", "48000", "-c", "1"])
        .arg(source)
        .arg(&dest)
        .output()
        .wrap_err("afconvert를 실행하지 못했습니다")?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&dest);
        return Err(eyre!("오디오를 압축하지 못했습니다: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(AudioUpload { path: dest, ext: "m4a".into(), temporary: true })
}

/// Windows: 운영체제에 쓸 만한 AAC 인코더 명령이 없어 순수 Rust FLAC(모노 16kHz)으로 압축한다. 1시간에 약 50MB
#[cfg(not(target_os = "macos"))]
fn compress(source: &Path, tmp: &Path) -> Result<AudioUpload> {
    let dest = tmp.with_extension("flac");
    encode_flac(source, &dest)?;
    Ok(AudioUpload { path: dest, ext: "flac".into(), temporary: true })
}

/// 모노 16kHz 16비트 FLAC. 전사 엔진이 쓰는 형식과 같아 다른 기기에서 받아 처리해도 결과가 같다
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn encode_flac(source: &Path, dest: &Path) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let samples: Vec<i32> = crate::engine::audio::load_mono_16k(source)?
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i32)
        .collect();
    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| eyre!("FLAC 설정 오류: {e:?}"))?;
    let source = flacenc::source::MemSource::from_samples(&samples, 1, 16, crate::engine::audio::SAMPLE_RATE as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).map_err(|e| eyre!("FLAC로 압축하지 못했습니다: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| eyre!("FLAC로 압축하지 못했습니다: {e:?}"))?;
    let mut bytes = sink.as_slice().to_vec();
    // flacenc는 STREAMINFO의 최소 블록 크기에 마지막(짧은) 블록 길이를 적는다. 규격은 마지막 블록을 빼고 세므로
    // 고정 크기 블록이면 최대와 같아야 하고, symphonia는 둘이 다르면 파일을 읽지 못했다("end of stream").
    // 바이트 0..4 "fLaC", 4..8 블록 헤더, 8..10 최소 블록 크기, 10..12 최대 블록 크기
    if bytes.len() > 12 && &bytes[..4] == b"fLaC" && bytes[4] & 0x7f == 0 {
        let max = [bytes[10], bytes[11]];
        bytes[8..10].copy_from_slice(&max);
    }
    std::fs::write(dest, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wav(path: &Path, seconds: u32) {
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..48_000 * seconds {
            let v = ((i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sori-sync-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn flac로_압축한_오디오를_전사_엔진이_같은_길이로_읽는다() {
        let wav = scratch("a.wav");
        sine_wav(&wav, 3);
        let flac = wav.with_extension("flac");

        encode_flac(&wav, &flac).unwrap();

        let samples = crate::engine::audio::load_mono_16k(&flac).unwrap();
        assert!((samples.len() as i64 - 48_000).abs() < 1600, "{}", samples.len());
        assert!(std::fs::metadata(&flac).unwrap().len() < std::fs::metadata(&wav).unwrap().len() / 4);
    }

    #[test]
    fn 데스크톱_녹음이_다른_데스크톱에_그대로_읽히고_제목과_메모가_오간다() {
        let cloud = FolderRemote::new(scratch("cloud"));
        let a = Store::new(scratch("a")).unwrap();
        let b = Store::new(scratch("b")).unwrap();
        let wav = scratch("src.wav");
        sine_wav(&wav, 1);
        let rec = a.import(&wav).unwrap();
        a.update(&rec.id, |r| r.notes.push(crate::store::Note { time: 0.5, text: "중요".into(), id: None, device: None })).unwrap();
        let local_a = DesktopLocal { store: &a, app: None, device: "A".into() };
        let local_b = DesktopLocal { store: &b, app: None, device: "B".into() };
        let run = |l: &DesktopLocal, s: &Store| {
            let report = sori_sync::sync(l, &cloud, "test", &s.data_file("sync-state.json"), &|_, _| {}).unwrap();
            assert!(report.errors.is_empty(), "{:?}", report.errors);
        };

        run(&local_a, &a);
        run(&local_b, &b);

        let got = b.get(&rec.id).unwrap();
        assert_eq!(got.title, rec.title);
        assert_eq!(got.number, rec.number);
        assert_eq!(got.status, Status::New);
        assert_eq!(got.notes.len(), 1);
        assert_eq!(got.notes[0].device.as_deref(), Some("A"));
        assert!(!b.audio_path(&got).unwrap().is_file());

        b.update(&rec.id, |r| r.title = "B에서 고친 제목".into()).unwrap();
        run(&local_b, &b);
        run(&local_a, &a);
        assert_eq!(a.get(&rec.id).unwrap().title, "B에서 고친 제목");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn afconvert로_압축한_m4a를_전사_엔진이_읽는다() {
        let wav = scratch("b.wav");
        sine_wav(&wav, 3);

        let upload = compress(&wav, &wav.with_extension("up")).unwrap();

        assert_eq!(upload.ext, "m4a");
        let samples = crate::engine::audio::load_mono_16k(&upload.path).unwrap();
        assert!((samples.len() as i64 - 48_000).abs() < 3200, "{}", samples.len());
    }
}
