use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use eyre::{eyre, Result, WrapErr};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::engine::Transcript;
use crate::store::Recording;

/// 구독 로그인된 CLI로 요약한다. 구독에는 API 키가 없어서 HTTP API 대신 CLI를 하위 프로세스로 부른다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    fn program(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: Provider,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub provider: Provider,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub markdown: String,
}

pub const DEFAULT_PROMPT: &str = "\
다음은 회의 녹음을 자동으로 받아 적은 기록이다. 화자는 자동으로 나눈 것이라 틀릴 수 있고, 받아 적기 오류도 있다. 문맥으로 보정해서 읽어라.

아래 형식의 한국어 마크다운으로 요약하라. 기록에 없는 내용은 지어내지 말고, 해당하는 내용이 없는 항목은 \"없음\"이라고 적어라.

## 한 줄 요약
## 주요 내용
## 결정 사항
## 할 일
담당자와 기한이 언급됐으면 함께 적는다.
## 남은 질문";

/// 요약이 이보다 오래 걸리면 멈춘다. 1시간 회의 요약이 보통 1분 안쪽이다
const TIMEOUT: Duration = Duration::from_secs(600);

/// 이 PATH로 CLI를 찾고 실행한다.
/// Finder에서 연 macOS 앱은 /usr/bin:/bin 정도만 물려받아 ~/.local/bin, Homebrew의 CLI를 못 찾는다.
/// 로그인 셸만으로는 .zshrc(nvm, ~/.local/bin 설정이 흔히 있는 곳)를 읽지 않아 대화형 로그인 셸(-ilc)로 묻는다.
/// 대화형 셸은 인사말 같은 다른 출력을 낼 수 있어 표시 문자열 사이의 값만 쓴다
fn login_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let inherited = std::env::var("PATH").unwrap_or_default();
        if cfg!(windows) {
            return Some(inherited);
        }
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let from_shell = std::process::Command::new(shell)
            .args(["-ilc", "printf '__SORI_PATH__%s__SORI_PATH__' \"$PATH\""])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|out| {
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.split("__SORI_PATH__").nth(1).map(str::to_string)
            })
            .unwrap_or_default();
        // 셸 설정에 없어도 흔한 설치 위치는 찾아본다
        let home = std::env::var("HOME").unwrap_or_default();
        let extra = [
            format!("{home}/.local/bin"),
            format!("{home}/.claude/local"),
            format!("{home}/.npm-global/bin"),
            "/opt/homebrew/bin".to_string(),
            "/usr/local/bin".to_string(),
        ];
        let mut parts: Vec<String> = Vec::new();
        for p in from_shell.split(':').chain(inherited.split(':')).map(str::to_string).chain(extra) {
            if !p.is_empty() && !parts.contains(&p) {
                parts.push(p);
            }
        }
        Some(parts.join(":"))
    })
    .as_deref()
}

pub fn find(provider: Provider) -> Option<PathBuf> {
    let path = login_path()?;
    let names: &[&str] = if cfg!(windows) {
        // npm 전역 설치는 .cmd, 공식 설치는 .exe
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(path).find_map(|dir| {
        names.iter().map(|ext| dir.join(format!("{}{ext}", provider.program()))).find(|p| p.is_file())
    })
}

pub fn statuses() -> Vec<ProviderStatus> {
    Provider::ALL
        .iter()
        .map(|&provider| ProviderStatus {
            provider,
            path: find(provider),
        })
        .collect()
}

pub fn format_transcript(recording: &Recording, transcript: &Transcript) -> String {
    let mut out = String::new();
    let mut last_speaker = None;
    for u in &transcript.utterances {
        if last_speaker == Some(u.speaker) {
            out.push(' ');
        } else {
            if last_speaker.is_some() {
                out.push('\n');
            }
            let name = recording
                .speaker_names
                .get(&u.speaker.to_string())
                .cloned()
                .unwrap_or_else(|| format!("화자 {}", u.speaker + 1));
            let name = match transcript.origins.get(&u.speaker) {
                Some(crate::engine::origin::Origin::Remote) => format!("{name}(원격)"),
                _ => name,
            };
            let secs = u.start as u32;
            out.push_str(&format!("[{:02}:{:02}] {name}: ", secs / 60, secs % 60));
            last_speaker = Some(u.speaker);
        }
        out.push_str(&u.text);
    }
    out
}

pub fn build_prompt(instructions: &str, recording: &Recording, transcript: &Transcript) -> String {
    format!(
        "{instructions}\n\n회의 제목: {}\n\n<transcript>\n{}\n</transcript>\n",
        recording.title,
        format_transcript(recording, transcript)
    )
}

/// 프롬프트를 표준 입력으로 넘겨 요약을 받는다. cancel이 오면 하위 프로세스를 죽인다
pub async fn run(provider: Provider, prompt: String, cancel: oneshot::Receiver<()>) -> Result<String> {
    let program = find(provider).ok_or_else(|| {
        eyre!(
            "{} CLI를 찾을 수 없습니다. 설치하고 로그인했는지 확인해 주세요",
            provider.program()
        )
    })?;

    // 빈 임시 폴더에서 돌려, 도구가 켜지더라도 다른 파일을 읽거나 쓰지 못하게 한다
    let workdir = std::env::temp_dir().join(format!("sori-summary-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workdir)?;
    let output_file = workdir.join("summary.md");

    let mut cmd = command_for(&program);
    match provider {
        Provider::Claude => {
            cmd.args(["-p", "--tools", "", "--no-session-persistence", "--strict-mcp-config", "--output-format", "text"]);
        }
        Provider::Codex => {
            cmd.args(["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never", "-o"])
                .arg(&output_file)
                .arg("-");
        }
    }
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    cmd.current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let result = async {
        let mut child = cmd.spawn().wrap_err_with(|| format!("{}를 실행하지 못했습니다", program.display()))?;
        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or_else(|| eyre!("표준 입력을 열지 못했습니다"))?;
        stdin.write_all(prompt.as_bytes()).await?;
        drop(stdin);

        let output = tokio::select! {
            out = child.wait_with_output() => out?,
            _ = cancel => {
                kill_tree(pid);
                eyre::bail!("요약을 취소했습니다")
            }
            _ = tokio::time::sleep(TIMEOUT) => {
                kill_tree(pid);
                eyre::bail!("요약이 {}분 안에 끝나지 않아 멈췄습니다", TIMEOUT.as_secs() / 60)
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.trim().is_empty() { stdout } else { stderr };
            eyre::bail!("{} 실행이 실패했습니다: {}", provider.program(), tail(&detail, 800));
        }
        let text = match provider {
            Provider::Claude => String::from_utf8_lossy(&output.stdout).into_owned(),
            Provider::Codex => std::fs::read_to_string(&output_file).wrap_err("codex 결과 파일을 읽지 못했습니다")?,
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            eyre::bail!("{}가 빈 요약을 돌려줬습니다", provider.program());
        }
        Ok(text)
    }
    .await;

    let _ = std::fs::remove_dir_all(&workdir);
    result
}

/// Rust 1.77부터 .cmd, .bat도 Command로 바로 실행하고 인자를 안전하게 이스케이프한다.
/// cmd /C로 직접 감싸면 사용자 폴더에 공백이 있을 때(C:\Users\Lee Young\...) 따옴표가 벗겨져 경로가 잘린다
fn command_for(program: &Path) -> Command {
    Command::new(program)
}

/// 취소나 시간 초과 때 하위 프로세스까지 끝낸다. Windows에서 .cmd는 cmd.exe가 node를 띄우므로
/// cmd.exe만 죽이면 node가 남는다
fn kill_tree(pid: Option<u32>) {
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn tail(text: &str, max: usize) -> String {
    let text = text.trim();
    let start = text.char_indices().rev().nth(max.saturating_sub(1)).map_or(0, |(i, _)| i);
    text[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::transcribe::Utterance;
    use std::collections::BTreeMap;

    fn utt(start: f32, speaker: i32, text: &str) -> Utterance {
        Utterance {
            start,
            end: start + 1.0,
            speaker,
            text: text.to_string(),
            lang: "ko".to_string(),
        }
    }

    fn recording(names: &[(&str, &str)]) -> Recording {
        let json = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "title": "주간 회의",
            "createdAt": "2026-10-28T00:00:00Z",
            "sourceName": "a.m4a",
            "audioFile": "audio.m4a",
            "status": "done",
            "error": null,
            "duration": null,
            "speakerNames": names.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(),
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn 같은_화자의_연속_발화는_한_줄로_묶는다() {
        let t = Transcript {
            duration: 10.0,
            turns: vec![],
            utterances: vec![utt(0.0, 0, "안녕하세요."), utt(1.5, 0, "시작할게요."), utt(65.0, 1, "네.")],
            origins: Default::default(),
        };

        let text = format_transcript(&recording(&[("0", "이영훈")]), &t);

        assert_eq!(text, "[00:00] 이영훈: 안녕하세요. 시작할게요.\n[01:05] 화자 2: 네.");
    }

    #[test]
    fn 긴_오류_메시지는_뒤쪽만_남긴다() {
        assert_eq!(tail("가나다라마", 2), "라마");
        assert_eq!(tail("짧음", 10), "짧음");
    }
}
