//! 앱 밖에서 들어오는 요청(전역 단축키, sori:// 딥링크)을 화면에 전달한다.
//! 녹음 상태와 처리 화면은 프론트가 들고 있으므로, 여기서는 할 일을 정해 이벤트로 넘기기만 한다.
//! @see docs/shortcuts-and-deep-links.md

use std::fs;
use std::path::{Path, PathBuf};
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
    /// 지금 녹음 중인 항목에 북마크를 남긴다
    Bookmark,
    ShowWindow,
    /// 녹음을 열고, time이 있으면 그 시각부터 재생할 준비를 한다
    Open { id: String, time: Option<f32> },
    /// process가 true면 가져온 뒤 기본 설정으로 바로 처리하고, false면 처리하지 않는다(MCP가 옵션을 붙여 따로 맡길 때).
    /// 없으면 설정의 "가져오면 자동 처리"를 따른다
    Import { paths: Vec<PathBuf>, process: Option<bool> },
    Search { query: String },
    /// 녹음을 받아 적는다(MCP, 스크립트용). 비어 있는 값은 앱 설정을 따른다
    #[serde(rename_all = "camelCase")]
    Process {
        id: String,
        num_speakers: Option<u32>,
        languages: Option<Vec<String>>,
        diarize: Option<bool>,
        /// 받아 적은 뒤 요약까지 할지. 없으면 설정의 "자동 요약"을 따른다
        summarize: Option<bool>,
        /// 요약 템플릿 id. 없으면 설정의 기본 템플릿
        template: Option<String>,
    },
    /// 이미 받아 적은 녹음을 다시 요약한다
    Summarize { id: String, template: Option<String> },
    /// 화자 이름을 붙인다. 빈 이름은 이름을 지운다. 화면의 이름 바꾸기와 같아 목소리 기억도 따른다
    NameSpeakers { id: String, names: Vec<(u32, String)> },
    /// AI가 뽑은 할 일 중 고른 것을 미리 알림에 넣는다
    #[serde(rename_all = "camelCase")]
    AddReminders { id: String, todo_ids: Vec<String> },
    /// 토큰 없는 녹음, 가져오기 링크를 막았다. 화면에 알린다
    Blocked { message: String },
}

impl Action {
    /// 창을 앞으로 가져올지. 녹음 토글은 회의 앱을 보는 중에 누르므로 창을 띄우지 않는다
    fn shows_window(&self) -> bool {
        // 처리 요청은 뒤에서 돈다. 다른 일을 하는 중에 창을 빼앗지 않는다
        !matches!(
            self,
            Action::ToggleRecording
                | Action::StartRecording
                | Action::StopRecording
                | Action::Bookmark
                | Action::Process { .. }
                | Action::Summarize { .. }
                | Action::NameSpeakers { .. }
                | Action::AddReminders { .. }
        )
    }
}

/// 전역 단축키 설정. 빈 문자열이면 등록하지 않는다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shortcuts {
    pub toggle_recording: String,
    pub show_window: String,
    pub bookmark: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        // macOS ⌥⇧⌘R, Windows Ctrl+Alt+Shift+R. 세 개를 함께 누르는 조합이라 다른 앱과 잘 겹치지 않는다
        Self {
            toggle_recording: "CommandOrControl+Alt+Shift+R".into(),
            show_window: "CommandOrControl+Alt+Shift+S".into(),
            bookmark: "CommandOrControl+Alt+Shift+B".into(),
        }
    }
}

#[derive(Default)]
pub struct Bus {
    /// 화면이 이벤트를 받을 준비가 되기 전에 들어온 요청. 딥링크로 앱을 처음 띄운 경우다
    pending: Mutex<Option<Vec<Action>>>,
    /// 등록한 단축키 id → 할 일
    shortcuts: Mutex<Vec<(u32, Action)>>,
    /// 녹음, 가져오기 링크가 함께 보내야 하는 값. 데이터 폴더의 link-token 파일에 있다
    token: Mutex<String>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(Some(Vec::new())),
            shortcuts: Mutex::new(Vec::new()),
            token: Mutex::new(String::new()),
        }
    }
}

/// sori가 가져올 수 있는 확장자. src/api.ts AUDIO_EXTENSIONS와 같다
const AUDIO_EXTENSIONS: [&str; 11] = ["m4a", "mp3", "wav", "aac", "flac", "ogg", "mp4", "mov", "mkv", "webm", "caf"];

/// 링크 토큰을 읽고, 없으면 만든다.
/// 웹 페이지도 sori:// 링크를 열 수 있어서, 녹음을 켜거나 파일을 가져오는 링크는 이 값을 함께 보내야 한다.
/// 이 컴퓨터의 사용자 권한으로 도는 프로그램(Raycast 등)만 파일을 읽을 수 있다
pub fn load_token(app: &AppHandle, data_dir: &Path) -> std::io::Result<()> {
    let path = data_dir.join("link-token");
    let token = match fs::read_to_string(&path) {
        Ok(t) if t.trim().len() >= 32 => t.trim().to_string(),
        _ => {
            let t = uuid::Uuid::new_v4().simple().to_string();
            fs::create_dir_all(data_dir)?;
            fs::write(&path, &t)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
            t
        }
    };
    *app.state::<Bus>().token.lock().unwrap() = token;
    Ok(())
}

/// 가져와도 되는 파일만 남긴다. 절대 경로의 실제 오디오 파일만 받고,
/// Windows 네트워크 경로(\\server\share)는 열기만 해도 인증 정보가 서버로 가므로 막는다
pub fn audio_files(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|p| {
            let s = p.to_string_lossy();
            let network = s.starts_with("\\\\") || s.starts_with("//");
            let audio = p
                .extension()
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()));
            p.is_absolute() && !network && audio && p.is_file()
        })
        .collect()
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
    let Some(action) = parse_url(url) else {
        eprintln!("알 수 없는 sori 링크: {url}");
        return;
    };
    let action = match action {
        Action::Import { paths, process } => Action::Import { paths: audio_files(paths), process },
        a => a,
    };
    if needs_token(&action) {
        let expected = app.state::<Bus>().token.lock().unwrap().clone();
        let given = url.query_pairs().find(|(k, _)| k == "token").map(|(_, v)| v.into_owned());
        if expected.is_empty() || given.as_deref() != Some(expected.as_str()) {
            dispatch(
                app,
                Action::Blocked {
                    message: "확인되지 않은 sori 링크가 녹음이나 가져오기를 요청해 막았습니다. 웹 페이지가 연 링크일 수 있습니다".into(),
                },
            );
            return;
        }
    }
    dispatch(app, action);
}

/// 마이크를 켜거나, 이 컴퓨터의 파일을 읽거나, 녹음의 전사와 요약을 덮어쓰는 요청. 열기, 검색, 창 앞으로는 토큰 없이 받는다.
/// 처리와 요약은 데이터를 바꾸고 CPU와 요약 사용량을 쓰므로 웹 페이지가 마음대로 부르지 못하게 한다
fn needs_token(action: &Action) -> bool {
    matches!(
        action,
        Action::ToggleRecording
            | Action::StartRecording
            | Action::StopRecording
            | Action::Bookmark
            | Action::Import { .. }
            | Action::Process { .. }
            | Action::Summarize { .. }
            | Action::NameSpeakers { .. }
            | Action::AddReminders { .. }
    )
}

/// 요약 템플릿 id(meeting, one-on-one 같은 영문 소문자, 숫자, -, _)만 받는다
fn template_id(v: Option<String>) -> Option<String> {
    v.filter(|t| (1..=40).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// "1", "true"면 참, "0", "false"면 거짓, 그 밖에는 값 없음
fn flag(v: Option<String>) -> Option<bool> {
    match v.as_deref() {
        Some("1" | "true") => Some(true),
        Some("0" | "false") => Some(false),
        _ => None,
    }
}

/// sori://record/toggle, sori://open?id=..&t=12.5, sori://import?path=..&path=..&process=1, sori://search?q=..,
/// sori://process?id=..&speakers=3&lang=ko&diarize=1&summarize=1&template=meeting, sori://summarize?id=..&template=..,
/// sori://speakers?id=..&name=0:영훈&name=1:시안, sori://reminders?id=..&todo=<할 일 id>
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
        ("record", "bookmark") => Some(Action::Bookmark),
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
                process: flag(query("process")),
            })
        }
        ("search", "") => Some(Action::Search {
            query: query("q").unwrap_or_default(),
        }),
        ("process", "") => {
            let languages: Vec<String> = url
                .query_pairs()
                .filter(|(k, _)| k == "lang")
                .map(|(_, v)| v.into_owned())
                // whisper 언어 코드(ko, en, zh, auto)만 받는다
                .filter(|l| (2..=5).contains(&l.len()) && l.chars().all(|c| c.is_ascii_lowercase()))
                .collect();
            Some(Action::Process {
                id: query("id").filter(|id| uuid::Uuid::parse_str(id).is_ok())?,
                num_speakers: query("speakers").and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=20).contains(n)),
                languages: (!languages.is_empty()).then_some(languages),
                diarize: flag(query("diarize")),
                summarize: flag(query("summarize")),
                template: template_id(query("template")),
            })
        }
        ("summarize", "") => Some(Action::Summarize {
            id: query("id").filter(|id| uuid::Uuid::parse_str(id).is_ok())?,
            template: template_id(query("template")),
        }),
        ("speakers", "") => {
            // name=<화자 번호>:<이름>. 이름 안의 :는 그대로 둔다
            let names: Vec<(u32, String)> = url
                .query_pairs()
                .filter(|(k, _)| k == "name")
                .filter_map(|(_, v)| {
                    let (n, name) = v.split_once(':')?;
                    let n = n.parse::<u32>().ok().filter(|n| *n < 100)?;
                    let name = name.trim();
                    (name.chars().count() <= 40).then(|| (n, name.to_string()))
                })
                .collect();
            if names.is_empty() {
                return None;
            }
            Some(Action::NameSpeakers { id: query("id").filter(|id| uuid::Uuid::parse_str(id).is_ok())?, names })
        }
        ("reminders", "") => {
            let todo_ids: Vec<String> = url
                .query_pairs()
                .filter(|(k, _)| k == "todo")
                .map(|(_, v)| v.into_owned())
                .filter(|t| !t.is_empty() && t.len() <= 64)
                .collect();
            if todo_ids.is_empty() {
                return None;
            }
            Some(Action::AddReminders { id: query("id").filter(|id| uuid::Uuid::parse_str(id).is_ok())?, todo_ids })
        }
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
        ("북마크", &shortcuts.bookmark, Action::Bookmark),
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
                process: Some(true),
            })
        );
    }

    #[test]
    fn 녹음과_가져오기와_처리만_토큰이_필요하다() {
        assert!(needs_token(&Action::StartRecording));
        assert!(needs_token(&Action::Import { paths: vec![], process: None }));
        assert!(needs_token(&Action::Summarize { id: String::new(), template: None }));
        assert!(needs_token(&Action::NameSpeakers { id: String::new(), names: vec![] }));
        assert!(needs_token(&Action::AddReminders { id: String::new(), todo_ids: vec![] }));
        assert!(needs_token(&Action::Process {
            id: String::new(),
            num_speakers: None,
            languages: None,
            diarize: None,
            summarize: None,
            template: None
        }));
        assert!(!needs_token(&Action::Search { query: String::new() }));
    }

    #[test]
    fn 처리_링크는_화자_수와_언어와_요약_여부를_읽는다() {
        let id = "0b8e7c52-3f7a-4c3e-9d55-2a7f0b9a1c11";

        let action = parse(&format!(
            "sori://process?id={id}&speakers=3&lang=ko&lang=en&diarize=1&summarize=0&template=one-on-one&token=x"
        ));

        assert_eq!(
            action,
            Some(Action::Process {
                id: id.into(),
                num_speakers: Some(3),
                languages: Some(vec!["ko".into(), "en".into()]),
                diarize: Some(true),
                summarize: Some(false),
                template: Some("one-on-one".into()),
            })
        );
    }

    #[test]
    fn 처리_링크의_빈_값은_설정을_따르도록_비워_둔다() {
        let id = "0b8e7c52-3f7a-4c3e-9d55-2a7f0b9a1c11";

        let action = parse(&format!("sori://process?id={id}&speakers=0&lang=../x&template=../x"));

        assert_eq!(
            action,
            Some(Action::Process {
                id: id.into(),
                num_speakers: None,
                languages: None,
                diarize: None,
                summarize: None,
                template: None
            })
        );
        assert_eq!(parse("sori://process?id=nope"), None);
        assert_eq!(
            parse(&format!("sori://summarize?id={id}")),
            Some(Action::Summarize { id: id.into(), template: None })
        );
    }

    #[test]
    fn 화자_이름_링크는_번호와_이름을_읽는다() {
        let id = "0b8e7c52-3f7a-4c3e-9d55-2a7f0b9a1c11";

        let action = parse(&format!(
            "sori://speakers?id={id}&name=0%3A%EC%98%81%ED%9B%88&name=2%3A&name=x%3Ay&name=1%3Aa%3Ab"
        ));

        assert_eq!(
            action,
            Some(Action::NameSpeakers {
                id: id.into(),
                names: vec![(0, "영훈".into()), (2, String::new()), (1, "a:b".into())],
            })
        );
        assert_eq!(parse(&format!("sori://speakers?id={id}")), None);
    }

    #[test]
    fn 미리_알림_링크는_할_일_id를_여러_개_받는다() {
        let id = "0b8e7c52-3f7a-4c3e-9d55-2a7f0b9a1c11";

        let action = parse(&format!("sori://reminders?id={id}&todo=a&todo=b"));

        assert_eq!(action, Some(Action::AddReminders { id: id.into(), todo_ids: vec!["a".into(), "b".into()] }));
        assert_eq!(parse(&format!("sori://reminders?id={id}")), None);
    }

    #[test]
    fn 처리_요청은_창을_띄우지_않는다() {
        assert!(!Action::Summarize { id: String::new(), template: None }.shows_window());
        assert!(!Action::NameSpeakers { id: String::new(), names: vec![] }.shows_window());
    }

    #[test]
    fn 네트워크_경로와_상대_경로와_오디오가_아닌_파일은_가져오지_않는다() {
        let dir = std::env::temp_dir().join(format!("sori-actions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("a.m4a");
        let text = dir.join("a.txt");
        fs::write(&audio, b"x").unwrap();
        fs::write(&text, b"x").unwrap();

        let kept = audio_files([
            audio.clone(),
            text,
            PathBuf::from("a.m4a"),
            PathBuf::from("\\\\server\\share\\a.m4a"),
            PathBuf::from("//server/share/a.m4a"),
        ]);

        assert_eq!(kept, vec![audio]);
        fs::remove_dir_all(dir).unwrap();
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
