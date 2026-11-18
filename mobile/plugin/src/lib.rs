//! sori iPhone 앱의 네이티브 기능(Swift, ios/Sources/SoriNativePlugin.swift).
//! 화면에서 직접 부르지 않고 앱의 Rust 코드가 부른다.
//! - 녹음: 구간 WAV로 쓰고 멈춘 뒤 AAC m4a로 합친다. 화면이 꺼져도 이어지고, 전화가 와도 앞부분을 잃지 않는다(docs/recording-safety.md)
//! - iCloud Drive 폴더: 파일 선택 창으로 고르고 접근 권한(bookmark)을 저장해 다음 실행에도 쓴다
//! - iCloud 파일 받기: 받지 않은 파일을 읽기 전에 NSFileCoordinator로 받아 온다

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Native(String),
    #[error("iPhone에서만 쓸 수 있습니다")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderState {
    pub recording: bool,
    /// 전화 같은 인터럽션으로 멈춰 있다. 녹음은 끝나지 않았고 이어지기를 기다린다
    #[serde(default)]
    pub paused: bool,
    /// 녹음한 초
    pub time: f64,
    /// 0~1 입력 크기
    pub level: f32,
}

/// 녹음이 멈췄다 이어진 곳. 통화 중에는 iOS가 마이크를 넘겨주지 않아 녹음할 수 없다(docs/recording-safety.md)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingGap {
    /// 이어 붙인 오디오에서 끊긴 위치(초)
    pub time: f64,
    /// 녹음하지 못한 시간(초)
    pub seconds: f64,
    /// 통화, 다른 앱 소리, 녹음 오류 등
    pub reason: String,
}

/// 녹음을 멈춘 결과
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    /// 녹음한 길이(초)
    pub duration: f64,
    #[serde(default)]
    pub gaps: Vec<RecordingGap>,
}

/// 구간 파일을 audio.m4a로 합친 결과(docs/recording-safety.md)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FinalizeOutcome {
    /// audio.m4a가 준비됐다
    Done,
    /// 녹음된 소리가 없다. 구간 파일은 남겨 둔다
    Empty,
    /// 구간 없이 옛 형식 audio.m4a만 있는데 읽히지 않는다(0.15.0 이하에서 끊긴 녹음)
    Broken,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finalized {
    pub outcome: FinalizeOutcome,
    /// 합친 오디오 길이(초)
    pub duration: f64,
}

/// 녹음을 시작한 곳. 못 받았으면 found가 false다
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub found: bool,
    #[serde(default)]
    pub lat: f64,
    #[serde(default)]
    pub lon: f64,
    #[serde(default)]
    pub accuracy: f64,
    #[serde(default)]
    pub place: Option<String>,
}

/// iPhone 미리 전사 결과. available이 false면 이 기기에서 못 한다(iOS 26 미만 등)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub utterances: Vec<PreviewUtterance>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewUtterance {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[cfg(mobile)]
mod native {
    use super::*;
    use serde::de::DeserializeOwned;
    use tauri::plugin::{PluginApi, PluginHandle};
    use tauri::AppHandle;

    #[cfg(target_os = "ios")]
    tauri::ios_plugin_binding!(init_plugin_sori_native);

    pub struct SoriNative<R: Runtime>(PluginHandle<R>);

    pub fn init<R: Runtime, C: DeserializeOwned>(_app: &AppHandle<R>, api: PluginApi<R, C>) -> crate::Result<SoriNative<R>> {
        #[cfg(target_os = "ios")]
        let handle = api.register_ios_plugin(init_plugin_sori_native).map_err(|e| Error::Native(e.to_string()))?;
        #[cfg(target_os = "android")]
        let handle: PluginHandle<R> = return Err(Error::Unsupported);
        Ok(SoriNative(handle))
    }

    /// 값 없이 끝나는 호출의 응답. 플러그인이 null이나 {}를 돌려줘도 읽힌다
    type Empty = serde_json::Value;

    #[derive(Deserialize)]
    struct MaybeFolder {
        path: Option<String>,
        name: Option<String>,
    }

    impl<R: Runtime> SoriNative<R> {
        fn call<T: DeserializeOwned>(&self, method: &str, payload: impl Serialize) -> crate::Result<T> {
            self.0.run_mobile_plugin(method, payload).map_err(|e| Error::Native(e.to_string()))
        }

        /// background: 단축어, 동작 버튼으로 앱 밖에서 시작한다. 다른 앱 소리와 섞어서 세션을 켠다
        pub fn start_recording(&self, path: &str, background: bool) -> crate::Result<()> {
            self.call::<Empty>("startRecording", serde_json::json!({ "path": path, "background": background })).map(|_| ())
        }

        /// 녹음한 길이와 멈췄다 이어진 곳
        pub fn stop_recording(&self) -> crate::Result<Stopped> {
            self.call("stopRecording", ())
        }

        pub fn recorder_state(&self) -> crate::Result<RecorderState> {
            self.call("recorderState", ())
        }

        /// 녹음 구간 파일을 path(audio.m4a)로 합친다. 다시 불러도 안전하다. 오류면 구간은 남아 있다
        pub fn finalize_recording(&self, path: &str) -> crate::Result<Finalized> {
            self.call("finalizeRecording", serde_json::json!({ "path": path }))
        }

        /// 파일 선택 창으로 폴더를 고른다. 취소하면 None
        pub fn pick_folder(&self) -> crate::Result<Option<Folder>> {
            let f: MaybeFolder = self.call("pickFolder", ())?;
            Ok(f.path.map(|path| Folder { path, name: f.name.unwrap_or_default() }))
        }

        /// 저장해 둔 폴더 접근 권한을 다시 연다. 고른 적이 없으면 None
        pub fn resolve_folder(&self) -> crate::Result<Option<Folder>> {
            let f: MaybeFolder = self.call("resolveFolder", ())?;
            Ok(f.path.map(|path| Folder { path, name: f.name.unwrap_or_default() }))
        }

        pub fn forget_folder(&self) -> crate::Result<()> {
            self.call::<Empty>("forgetFolder", ()).map(|_| ())
        }

        /// 받지 않은 iCloud 파일을 받아 온다. 이미 있으면 바로 끝난다
        pub fn materialize(&self, path: &str) -> crate::Result<()> {
            self.call::<Empty>("materialize", serde_json::json!({ "path": path })).map(|_| ())
        }

        /// 다른 기기가 고친 뒤 아직 내려오지 않은 작은 파일을 받아 온다. 받은 개수
        pub fn refresh_icloud(&self, path: &str) -> crate::Result<u32> {
            #[derive(serde::Deserialize)]
            struct Count {
                count: u32,
            }
            self.call::<Count>("refreshIcloud", serde_json::json!({ "path": path })).map(|c| c.count)
        }

        /// 지금 위치와 주소. 권한이 없거나 못 받으면 found가 false다(최대 약 18초)
        pub fn current_location(&self) -> crate::Result<Fix> {
            self.call("currentLocation", ())
        }

        /// 기기 안에서 미리 전사한다(iOS 26 SpeechAnalyzer). 몇 분 걸릴 수 있다
        pub fn transcribe_preview(&self, path: &str) -> crate::Result<Preview> {
            self.call("transcribePreview", serde_json::json!({ "path": path }))
        }

        /// iOS 알림창으로 묻는다. 확인을 누르면 true. 웹뷰의 window.confirm은 iOS에서 창 없이 늘 false다
        pub fn confirm(&self, title: &str, message: Option<&str>, ok: &str, destructive: bool) -> crate::Result<bool> {
            #[derive(serde::Deserialize)]
            struct Answer {
                ok: bool,
            }
            self.call::<Answer>(
                "confirm",
                serde_json::json!({ "title": title, "message": message, "ok": ok, "destructive": destructive }),
            )
            .map(|a| a.ok)
        }

        /// 가벼운 진동(당겨서 새로고침)
        pub fn haptic(&self) -> crate::Result<()> {
            self.call::<Empty>("haptic", serde_json::json!({})).map(|_| ())
        }

        /// 지도 앱이나 Safari로 링크를 연다
        pub fn open_url(&self, url: &str) -> crate::Result<()> {
            self.call::<Empty>("openUrl", serde_json::json!({ "url": url })).map(|_| ())
        }
    }
}

/// 데스크톱에서 `cargo check`할 때만 쓴다. 모든 기능이 Unsupported다
#[cfg(desktop)]
mod native {
    use super::*;

    pub struct SoriNative<R: Runtime>(pub(crate) std::marker::PhantomData<fn() -> R>);

    impl<R: Runtime> SoriNative<R> {
        pub fn start_recording(&self, _: &str, _: bool) -> crate::Result<()> {
            Err(Error::Unsupported)
        }
        pub fn stop_recording(&self) -> crate::Result<Stopped> {
            Err(Error::Unsupported)
        }
        pub fn recorder_state(&self) -> crate::Result<RecorderState> {
            Err(Error::Unsupported)
        }
        pub fn finalize_recording(&self, _: &str) -> crate::Result<Finalized> {
            Err(Error::Unsupported)
        }
        pub fn pick_folder(&self) -> crate::Result<Option<Folder>> {
            Err(Error::Unsupported)
        }
        pub fn resolve_folder(&self) -> crate::Result<Option<Folder>> {
            Ok(None)
        }
        pub fn forget_folder(&self) -> crate::Result<()> {
            Ok(())
        }
        pub fn materialize(&self, _: &str) -> crate::Result<()> {
            Ok(())
        }
        pub fn refresh_icloud(&self, _: &str) -> crate::Result<u32> {
            Ok(0)
        }
        pub fn open_url(&self, _: &str) -> crate::Result<()> {
            Err(Error::Unsupported)
        }
        pub fn haptic(&self) -> crate::Result<()> {
            Ok(())
        }
        pub fn confirm(&self, _: &str, _: Option<&str>, _: &str, _: bool) -> crate::Result<bool> {
            Err(Error::Unsupported)
        }
        pub fn current_location(&self) -> crate::Result<Fix> {
            Ok(Fix::default())
        }
        pub fn transcribe_preview(&self, _: &str) -> crate::Result<Preview> {
            Ok(Preview::default())
        }
    }
}

pub use native::SoriNative;

pub trait SoriNativeExt<R: Runtime> {
    fn sori_native(&self) -> &SoriNative<R>;
}

impl<R: Runtime, T: Manager<R>> SoriNativeExt<R> for T {
    fn sori_native(&self) -> &SoriNative<R> {
        self.state::<SoriNative<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("sori-native")
        .setup(|app, _api| {
            #[cfg(mobile)]
            let plugin = native::init(app, _api)?;
            #[cfg(desktop)]
            let plugin = native::SoriNative::<R>(std::marker::PhantomData);
            app.manage(plugin);
            Ok(())
        })
        .build()
}
