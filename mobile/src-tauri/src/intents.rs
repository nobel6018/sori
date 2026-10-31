//! 앱 타깃 Swift(단축어 인텐트, Live Activity)와 Rust를 잇는 C 함수.
//! 인텐트는 Tauri 플러그인이 아니라 Xcode 앱 타깃(gen/apple/Sources)에 있어야 단축어 앱이 찾는다.
//! 녹음 상태는 여기서도 lib.rs의 AppState 하나로 다룬다.
//! @see docs/mobile.md

use std::ffi::{c_char, CStr, CString};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();
static HOOKS: OnceLock<ActivityHooks> = OnceLock::new();

/// Swift가 넘겨준 Live Activity 시작, 끝 함수.
/// Rust가 Swift 심볼을 링크로 직접 부르면 함께 만드는 cdylib가 미해결 심볼로 깨져, 실행할 때 포인터로 받는다
struct ActivityHooks {
    /// 띄웠으면 null, 못 띄웠으면 오류 문구(Swift가 가지고 있어 바로 복사한다)
    started: extern "C" fn() -> *const c_char,
    stopped: extern "C" fn(),
}

pub(crate) fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

/// Live Activity를 띄우거나, 단축어가 이미 띄웠으면 Ok. 못 띄우면 iOS가 준 오류
pub(crate) fn activity_started() -> Result<(), String> {
    let Some(hooks) = HOOKS.get() else { return Err("iPhone이 아닙니다".into()) };
    let error = (hooks.started)();
    if error.is_null() {
        return Ok(());
    }
    Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned())
}

pub(crate) fn activity_stopped() {
    if let Some(hooks) = HOOKS.get() {
        (hooks.stopped)();
    }
}

/// 꺼져 있던 앱을 단축어가 띄웠으면 Tauri setup이 끝날 때까지 기다린다
fn app() -> Result<&'static AppHandle, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(app) = APP.get() {
            return Ok(app);
        }
        if Instant::now() > deadline {
            return Err("sori가 준비되지 않았습니다. 앱을 한 번 열었다가 다시 시도해 주세요".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 성공하면 null, 실패하면 오류 문구. 문구는 Swift가 sori_intent_free로 돌려준다
fn into_c(result: Result<(), String>) -> *mut c_char {
    match result {
        Ok(()) => std::ptr::null_mut(),
        Err(e) => CString::new(e.replace('\0', "")).unwrap_or_default().into_raw(),
    }
}

/// main.mm이 앱을 띄우기 전에 부른다
#[unsafe(no_mangle)]
pub extern "C" fn sori_set_activity_hooks(started: extern "C" fn() -> *const c_char, stopped: extern "C" fn()) {
    let _ = HOOKS.set(ActivityHooks { started, stopped });
}

/// 메인 스레드에서 부르면 안 된다. 메인 스레드에서 도는 Swift 녹음기를 기다리므로 앱이 멈춘다
#[unsafe(no_mangle)]
pub extern "C" fn sori_intent_start_recording() -> *mut c_char {
    into_c(app().and_then(|app| crate::begin_recording(app, true).map(|_| ())))
}

/// 메인 스레드에서 부르면 안 된다(sori_intent_start_recording과 같은 이유)
#[unsafe(no_mangle)]
pub extern "C" fn sori_intent_stop_recording() -> *mut c_char {
    into_c(app().and_then(|app| crate::end_recording(app).map(|_| ())))
}

#[unsafe(no_mangle)]
pub extern "C" fn sori_intent_is_recording() -> bool {
    app().map(crate::is_recording).unwrap_or(false)
}

/// # Safety
/// sori_intent_start_recording, sori_intent_stop_recording이 돌려준 포인터만 넘긴다
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sori_intent_free(message: *mut c_char) {
    if !message.is_null() {
        drop(unsafe { CString::from_raw(message) });
    }
}
