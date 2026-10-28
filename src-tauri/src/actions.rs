//! 앱 밖에서 들어오는 요청(전역 단축키, sori:// 딥링크)을 화면에 전달한다.
//! 녹음 상태와 처리 화면은 프론트가 들고 있으므로, 여기서는 할 일을 정해 이벤트로 넘기기만 한다.
//! @see docs/shortcuts-and-deep-links.md

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub const EVENT: &str = "app-action";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    ToggleRecording,
    StartRecording,
    StopRecording,
    ShowWindow,
    /// 녹음을 열고, time이 있으면 그 시각부터 재생할 준비를 한다
    Open { id: String, time: Option<f32> },
    /// process가 true면 가져온 뒤 기본 설정으로 바로 처리한다
    Import { paths: Vec<PathBuf>, process: bool },
    Search { query: String },
}

impl Action {
    /// 창을 앞으로 가져올지. 녹음 토글은 회의 앱을 보는 중에 누르므로 창을 띄우지 않는다
    fn shows_window(&self) -> bool {
        !matches!(self, Action::ToggleRecording | Action::StartRecording | Action::StopRecording)
    }
}

/// 전역 단축키 설정. 빈 문자열이면 등록하지 않는다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shortcuts {
    pub toggle_recording: String,
    pub show_window: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        // macOS ⌥⇧⌘R, Windows Ctrl+Alt+Shift+R. 세 개를 함께 누르는 조합이라 다른 앱과 잘 겹치지 않는다
        Self {
            toggle_recording: "CommandOrControl+Alt+Shift+R".into(),
            show_window: "CommandOrControl+Alt+Shift+S".into(),
        }
    }
}

#[derive(Default)]
pub struct Bus {
    /// 화면이 이벤트를 받을 준비가 되기 전에 들어온 요청. 딥링크로 앱을 처음 띄운 경우다
    pending: Mutex<Option<Vec<Action>>>,
    /// 등록한 단축키 id → 할 일
    shortcuts: Mutex<Vec<(u32, Action)>>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(Some(Vec::new())),
            shortcuts: Mutex::new(Vec::new()),
        }
    }
}

pub fn dispatch(app: &AppHandle, action: Action) {
    if action.shows_window() {
        show_main(app);
    }
    let bus = app.state::<Bus>();
    let mut pending = bus.pending.lock().unwrap();
    match pending.as_mut() {
        Some(queue) => queue.push(action),
        None => {
            let _ = app.emit(EVENT, action);
        }
    }
}

/// 화면이 구독을 마쳤다. 그동안 쌓인 요청을 돌려주고, 이후로는 바로 이벤트로 보낸다
pub fn take_pending(app: &AppHandle) -> Vec<Action> {
    app.state::<Bus>().pending.lock().unwrap().take().unwrap_or_default()
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn handle_url(app: &AppHandle, url: &Url) {
    match parse_url(url) {
        Some(action) => dispatch(app, action),
        None => eprintln!("알 수 없는 sori 링크: {url}"),
    }
}

/// sori://record/toggle, sori://open?id=..&t=12.5, sori://import?path=..&path=..&process=1, sori://search?q=..
pub fn parse_url(url: &Url) -> Option<Action> {
    if url.scheme() != "sori" {
        return None;
    }
    let query = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
    let path = url.path().trim_matches('/');
    match (url.host_str().unwrap_or(""), path) {
        ("record", "toggle") => Some(Action::ToggleRecording),
        ("record", "start") => Some(Action::StartRecording),
        ("record", "stop") => Some(Action::StopRecording),
        ("show", "") => Some(Action::ShowWindow),
        ("open", "") => Some(Action::Open {
            id: query("id").filter(|id| uuid::Uuid::parse_str(id).is_ok())?,
            time: query("t").and_then(|t| t.parse::<f32>().ok()).filter(|t| t.is_finite() && *t >= 0.0),
        }),
        ("import", "") => {
            let paths: Vec<PathBuf> = url
                .query_pairs()
                .filter(|(k, _)| k == "path")
                .map(|(_, v)| PathBuf::from(v.into_owned()))
                .collect();
            if paths.is_empty() {
                return None;
            }
            Some(Action::Import {
                paths,
                process: query("process").is_some_and(|v| v == "1" || v == "true"),
            })
        }
        ("search", "") => Some(Action::Search {
            query: query("q").unwrap_or_default(),
        }),
        _ => None,
    }
}

/// 전역 단축키를 새로 등록한다. 하나라도 실패하면 이전 설정으로 되돌리고 이유를 알려 준다
pub fn apply_shortcuts(app: &AppHandle, next: &Shortcuts, previous: Option<&Shortcuts>) -> Result<(), String> {
    match register(app, next) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(prev) = previous {
                let _ = register(app, prev);
            }
            Err(e)
        }
    }
}

fn register(app: &AppHandle, shortcuts: &Shortcuts) -> Result<(), String> {
    let manager = app.global_shortcut();
    let bus = app.state::<Bus>();
    let _ = manager.unregister_all();
    bus.shortcuts.lock().unwrap().clear();

    let entries = [
        ("녹음 시작/멈춤", &shortcuts.toggle_recording, Action::ToggleRecording),
        ("sori 창 앞으로", &shortcuts.show_window, Action::ShowWindow),
    ];
    for (label, accelerator, action) in entries {
        if accelerator.trim().is_empty() {
            continue;
        }
        let shortcut: Shortcut = accelerator
            .parse()
            .map_err(|e| format!("{label} 단축키를 읽지 못했습니다({accelerator}): {e}"))?;
        manager
            .register(shortcut)
            .map_err(|e| format!("{label} 단축키({accelerator})를 등록하지 못했습니다. 다른 앱이 쓰고 있을 수 있습니다: {e}"))?;
        bus.shortcuts.lock().unwrap().push((shortcut.id(), action));
    }
    Ok(())
}

pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut) {
    let action = app
        .state::<Bus>()
        .shortcuts
        .lock()
        .unwrap()
        .iter()
        .find(|(id, _)| *id == shortcut.id())
        .map(|(_, a)| a.clone());
    if let Some(action) = action {
        dispatch(app, action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Option<Action> {
        parse_url(&Url::parse(s).unwrap())
    }

    #[test]
    fn 녹음_링크를_읽는다() {
        assert_eq!(parse("sori://record/toggle"), Some(Action::ToggleRecording));
        assert_eq!(parse("sori://record/stop/"), Some(Action::StopRecording));
    }

    #[test]
    fn 열기_링크는_시각을_함께_읽는다() {
        let id = "0b8e7c52-3f7a-4c3e-9d55-2a7f0b9a1c11";

        let action = parse(&format!("sori://open?id={id}&t=12.5"));

        assert_eq!(action, Some(Action::Open { id: id.into(), time: Some(12.5) }));
    }

    #[test]
    fn 열기_링크의_잘못된_id는_버린다() {
        assert_eq!(parse("sori://open?id=../../etc"), None);
    }

    #[test]
    fn 가져오기_링크는_경로를_여러_개_받는다() {
        let action = parse("sori://import?path=%2FUsers%2Fa%2F%ED%9A%8C%EC%9D%98.m4a&path=%2Ftmp%2Fb.wav&process=1");

        assert_eq!(
            action,
            Some(Action::Import {
                paths: vec![PathBuf::from("/Users/a/회의.m4a"), PathBuf::from("/tmp/b.wav")],
                process: true,
            })
        );
    }

    #[test]
    fn 다른_스킴과_모르는_경로는_무시한다() {
        assert_eq!(parse("https://record/toggle"), None);
        assert_eq!(parse("sori://record/delete"), None);
        assert_eq!(parse("sori://import"), None);
    }

    #[test]
    fn 기본_단축키는_읽을_수_있다() {
        let d = Shortcuts::default();

        assert!(d.toggle_recording.parse::<Shortcut>().is_ok());
        assert!(d.show_window.parse::<Shortcut>().is_ok());
    }

    #[test]
    fn 설정_화면이_만드는_표기를_모두_읽는다() {
        // src/shortcuts.ts acceleratorFromEvent가 만드는 키 이름들
        for s in [
            "CommandOrControl+Control+Alt+Shift+KeyR",
            "CommandOrControl+R",
            "Super+Alt+5",
            "CommandOrControl+F12",
            "CommandOrControl+Space",
            "CommandOrControl+ArrowUp",
            "CommandOrControl+Comma",
            "CommandOrControl+Backquote",
            "CommandOrControl+BracketLeft",
            "CommandOrControl+Slash",
        ] {
            assert!(s.parse::<Shortcut>().is_ok(), "{s}");
        }
    }
}
