//! 회의 앱이 마이크를 쓰기 시작하면 "녹음할까요?"를 묻는다.
//! macOS는 Core Audio의 프로세스 목록(14.0 이상)에서 마이크를 쓰는 앱을 찾고,
//! Windows는 설정의 "마이크를 사용 중인 앱" 기록(CapabilityAccessManager)을 읽는다.
//! @see docs/meeting-detection.md

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// 마이크를 쓰는 앱 하나. key는 같은 앱을 알아보는 값(번들 id나 실행 파일 경로)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MicApp {
    pub key: String,
    pub name: String,
}

/// 알아보는 회의 앱. 번들 id나 실행 파일 경로(소문자)에 이 글자가 들어 있으면 그 앱이다.
/// 브라우저는 Meet인지 다른 사이트인지 알 수 없어 "Google Meet 등"으로 묻는다
const KNOWN: [(&str, &str); 22] = [
    ("us.zoom.", "Zoom"),
    ("zoom.exe", "Zoom"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("msteams", "Microsoft Teams"),
    ("ms-teams.exe", "Microsoft Teams"),
    ("teams.exe", "Microsoft Teams"),
    ("webex", "Webex"),
    ("com.tinyspeck.slackmacgap", "Slack 허들"),
    ("slack.exe", "Slack 허들"),
    ("com.hnc.discord", "Discord"),
    ("discord.exe", "Discord"),
    ("com.apple.facetime", "FaceTime"),
    ("com.kakao.kakaotalkmac", "카카오톡 통화"),
    ("kakaotalk.exe", "카카오톡 통화"),
    ("com.google.chrome", "Chrome (Google Meet 등)"),
    ("chrome.exe", "Chrome (Google Meet 등)"),
    ("com.apple.webkit.gpu", "Safari (Google Meet 등)"),
    ("company.thebrowser.browser", "Arc (Google Meet 등)"),
    ("com.microsoft.edgemac", "Edge (Google Meet 등)"),
    ("msedge.exe", "Edge (Google Meet 등)"),
    ("firefox", "Firefox (Google Meet 등)"),
    ("whale", "Whale (Google Meet 등)"),
];

pub fn known_name(key: &str) -> Option<&'static str> {
    let key = key.to_lowercase();
    KNOWN.iter().find(|(pattern, _)| key.contains(pattern)).map(|(_, name)| *name)
}

#[cfg(target_os = "macos")]
pub use platform::audio_processes;

/// 지금 마이크를 쓰는 회의 앱. sori 자신은 뺀다
pub fn meeting_apps_using_mic() -> Vec<MicApp> {
    let mut apps: Vec<MicApp> = platform::apps_using_mic()
        .into_iter()
        .filter_map(|key| known_name(&key).map(|name| MicApp { key, name: name.to_string() }))
        .collect();
    apps.sort();
    // 같은 앱의 여러 프로세스(Chrome 도우미 등)는 하나로 본다
    apps.dedup_by(|a, b| a.name == b.name);
    apps
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Prompt {
    /// 회의 앱이 마이크를 쓰기 시작했다. 녹음할지 묻는다
    Start { app: String },
    /// 질문으로 시작한 녹음인데 그 앱이 마이크를 놓았다. 멈출지 묻는다
    Stop { app: String },
}

/// 회의가 끝났다고 보기 전에 기다리는 시간. 통화 중 잠깐 음소거해도 마이크를 놓는 앱이 있다
const STOP_GRACE: Duration = Duration::from_secs(15);

/// 몇 초마다 마이크를 쓰는 앱 목록을 받아 언제 물을지 정한다. 시스템 호출과 떼어 두어 시험할 수 있게 한다
#[derive(Default)]
pub struct Detector {
    /// 직전에 마이크를 쓰던 앱(이름)
    active: BTreeSet<String>,
    /// 질문으로 녹음을 시작하게 한 앱과, 그 앱이 마이크를 놓은 시각
    started_by: Option<(String, Option<Instant>)>,
}

impl Detector {
    pub fn update(&mut self, apps: &[MicApp], recording: bool, now: Instant) -> Option<Prompt> {
        let names: BTreeSet<String> = apps.iter().map(|a| a.name.clone()).collect();
        let started: Vec<String> = names.difference(&self.active).cloned().collect();
        self.active = names.clone();

        if !recording {
            // 녹음이 끝났으면(직접 멈췄어도) 더 따라가지 않는다
            self.started_by = None;
            return started.into_iter().next().map(|app| Prompt::Start { app });
        }
        if let Some((app, released)) = self.started_by.as_mut() {
            if names.contains(app.as_str()) {
                *released = None;
            } else {
                let since = *released.get_or_insert(now);
                if now.duration_since(since) >= STOP_GRACE {
                    let app = app.clone();
                    self.started_by = None;
                    return Some(Prompt::Stop { app });
                }
            }
        }
        None
    }

    /// 사용자가 질문에 답해 녹음을 시작했다. 그 앱이 마이크를 놓으면 멈출지 묻는다
    pub fn recording_started_for(&mut self, app: String) {
        self.started_by = Some((app, None));
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
        kAudioObjectSystemObject, kAudioProcessPropertyBundleID, kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyPID,
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    };
    use objc2_core_foundation::{CFRetained, CFString};

    fn address(selector: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    fn get<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Option<T> {
        let addr = address(selector);
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        (status == 0).then_some(value)
    }

    fn process_list() -> Vec<AudioObjectID> {
        let addr = address(kAudioHardwarePropertyProcessObjectList);
        let system = kAudioObjectSystemObject as AudioObjectID;
        let mut size = 0u32;
        let status = unsafe { AudioObjectGetPropertyDataSize(system, NonNull::from(&addr), 0, std::ptr::null(), NonNull::from(&mut size)) };
        if status != 0 || size == 0 {
            return Vec::new();
        }
        let mut ids = vec![0 as AudioObjectID; size as usize / std::mem::size_of::<AudioObjectID>()];
        let status = unsafe {
            AudioObjectGetPropertyData(
                system,
                NonNull::from(&addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(ids.as_mut_ptr().cast::<c_void>()).unwrap(),
            )
        };
        if status != 0 {
            return Vec::new();
        }
        ids.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
        ids
    }

    fn bundle_id(object: AudioObjectID) -> Option<String> {
        let addr = address(kAudioProcessPropertyBundleID);
        let mut value: *const CFString = std::ptr::null();
        let mut size = std::mem::size_of::<*const CFString>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        let ptr = NonNull::new(value as *mut CFString)?;
        if status != 0 {
            return None;
        }
        // 이 속성은 받은 쪽이 해제해야 하는 문자열(Create 규칙)을 돌려준다
        let s = unsafe { CFRetained::from_raw(ptr) };
        Some(s.to_string())
    }

    /// 오디오를 쓰는 모든 프로세스와 마이크 사용 여부. 확인용 예제(examples/meeting_check.rs)가 쓴다
    pub fn audio_processes() -> Vec<(String, bool)> {
        process_list()
            .into_iter()
            .filter_map(|p| Some((bundle_id(p)?, get::<u32>(p, kAudioProcessPropertyIsRunningInput).unwrap_or(0) != 0)))
            .collect()
    }

    /// 마이크 입력을 쓰는 프로세스의 번들 id
    pub fn apps_using_mic() -> Vec<String> {
        let me = std::process::id() as i32;
        process_list()
            .into_iter()
            .filter(|&p| get::<u32>(p, kAudioProcessPropertyIsRunningInput).unwrap_or(0) != 0)
            .filter(|&p| get::<i32>(p, kAudioProcessPropertyPID) != Some(me))
            .filter_map(bundle_id)
            .collect()
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    const ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open(parent: HKEY, path: &str) -> Option<Key> {
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe { RegOpenKeyExW(parent, wide(path).as_ptr(), 0, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then(|| Key(key))
    }

    fn subkeys(key: &Key) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = [0u16; 512];
        for i in 0.. {
            let mut len = buf.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(key.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            if status != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }

    fn qword(key: &Key, name: &str) -> Option<u64> {
        let mut value = 0u64;
        let mut size = 8u32;
        let status = unsafe {
            RegQueryValueExW(key.0, wide(name).as_ptr(), std::ptr::null(), std::ptr::null_mut(), (&mut value as *mut u64).cast(), &mut size)
        };
        (status == ERROR_SUCCESS).then_some(value)
    }

    /// 시작 시각은 있고 끝난 시각이 0이면 지금 쓰는 중이다
    fn in_use(key: &Key) -> bool {
        qword(key, "LastUsedTimeStart").unwrap_or(0) != 0 && qword(key, "LastUsedTimeStop") == Some(0)
    }

    /// 마이크를 쓰는 앱. 일반 앱은 실행 파일 경로(C:#Program Files#Zoom#bin#Zoom.exe), 스토어 앱은 패키지 이름
    pub fn apps_using_mic() -> Vec<String> {
        let Some(root) = open(HKEY_CURRENT_USER, ROOT) else { return Vec::new() };
        let me = std::env::current_exe().ok().map(|p| p.to_string_lossy().replace('\\', "#").to_lowercase());
        let mut out = Vec::new();
        for name in subkeys(&root) {
            let Some(key) = open(root.0, &name) else { continue };
            if name == "NonPackaged" {
                for app in subkeys(&key) {
                    if me.as_deref() == Some(app.to_lowercase().as_str()) {
                        continue;
                    }
                    if open(key.0, &app).is_some_and(|k| in_use(&k)) {
                        out.push(app);
                    }
                }
            } else if in_use(&key) {
                out.push(name);
            }
        }
        out
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn apps_using_mic() -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> MicApp {
        MicApp { key: name.into(), name: name.into() }
    }

    #[test]
    fn 번들_id와_실행_파일_경로로_회의_앱을_알아본다() {
        assert_eq!(known_name("us.zoom.xos"), Some("Zoom"));
        assert_eq!(known_name("C:#Program Files#Zoom#bin#Zoom.exe"), Some("Zoom"));
        assert_eq!(known_name("MSTeams_8wekyb3d8bbwe"), Some("Microsoft Teams"));
        assert_eq!(known_name("com.google.Chrome.helper"), Some("Chrome (Google Meet 등)"));
        assert_eq!(known_name("com.apple.VoiceMemos"), None);
    }

    #[test]
    fn 새로_마이크를_쓰기_시작한_앱만_묻는다() {
        let mut d = Detector::default();
        let t = Instant::now();

        assert_eq!(d.update(&[app("Zoom")], false, t), Some(Prompt::Start { app: "Zoom".into() }));
        // 계속 쓰는 중이면 다시 묻지 않는다
        assert_eq!(d.update(&[app("Zoom")], false, t), None);
        assert_eq!(d.update(&[], false, t), None);
        assert_eq!(d.update(&[app("Zoom")], false, t), Some(Prompt::Start { app: "Zoom".into() }));
    }

    #[test]
    fn 녹음_중에는_시작을_묻지_않는다() {
        let mut d = Detector::default();

        assert_eq!(d.update(&[app("Zoom")], true, Instant::now()), None);
    }

    #[test]
    fn 질문으로_시작한_녹음은_앱이_마이크를_놓고_잠시_뒤에_멈출지_묻는다() {
        let mut d = Detector::default();
        let t = Instant::now();
        d.update(&[app("Zoom")], false, t);
        d.recording_started_for("Zoom".into());

        assert_eq!(d.update(&[app("Zoom")], true, t), None);
        assert_eq!(d.update(&[], true, t + Duration::from_secs(1)), None);
        // 잠깐 놓았다 다시 쓰면 기다림을 처음부터 센다
        assert_eq!(d.update(&[app("Zoom")], true, t + Duration::from_secs(5)), None);
        assert_eq!(d.update(&[], true, t + Duration::from_secs(6)), None);
        assert_eq!(
            d.update(&[], true, t + Duration::from_secs(6) + STOP_GRACE),
            Some(Prompt::Stop { app: "Zoom".into() })
        );
        assert_eq!(d.update(&[], true, t + Duration::from_secs(60)), None);
    }

    #[test]
    fn 직접_시작한_녹음은_멈추라고_묻지_않는다() {
        let mut d = Detector::default();
        let t = Instant::now();
        d.update(&[app("Zoom")], true, t);

        assert_eq!(d.update(&[], true, t + Duration::from_secs(120)), None);
    }
}
