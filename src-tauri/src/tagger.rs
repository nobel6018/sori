//! AI 정리. 받아 적기가 끝난 녹음에 구독 CLI(claude, codex) 한 번으로 제목, 태그, 할 일 후보를 붙인다.
//! 쓰던 태그를 먼저 고르게 해 태그가 마구 늘지 않게 하고, 사람이 고친 제목과 태그는 덮어쓰지 않는다.
//! 할 일은 후보로만 두고, 사용자가 확인한 것만 미리 알림에 넣는다(calendar.rs).
//! @see docs/organize.md

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use eyre::{eyre, Result};
use serde::Deserialize;

use crate::engine::Transcript;
use crate::store::{CalendarEvent, Recording};
use crate::summarize;

/// AI가 붙이는 태그 수
const MAX_AI_TAGS: usize = 3;
/// AI가 뽑는 할 일 수
const MAX_TODOS: usize = 10;
/// 요약이 있을 때 함께 넘기는 전사 앞부분 글자 수
const EXCERPT_WITH_SUMMARY: usize = 3_000;
/// 요약이 없을 때 넘기는 전사 앞부분과 뒷부분 글자 수. 태그를 고르는 데 전사 전체는 필요 없다
const EXCERPT_HEAD: usize = 8_000;
const EXCERPT_TAIL: usize = 4_000;

/// 같은 녹음을 동시에 두 번 분류하지 않는다(자동 분류와 일괄 분류가 겹칠 때)
static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub struct RunningGuard(String);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if let Some(set) = RUNNING.lock().unwrap().as_mut() {
            set.remove(&self.0);
        }
    }
}

/// 이미 분류 중이면 None
/// 지금 AI로 정리 중인 녹음 id. 업데이트 전에 진행 중인 작업을 알린다(lib.rs busy_tasks)
pub fn running() -> Vec<String> {
    RUNNING.lock().unwrap().as_ref().map(|s| s.iter().cloned().collect()).unwrap_or_default()
}

pub fn begin(id: &str) -> Option<RunningGuard> {
    let mut lock = RUNNING.lock().unwrap();
    let set = lock.get_or_insert_with(HashSet::new);
    set.insert(id.to_string()).then(|| RunningGuard(id.to_string()))
}

/// 다른 녹음에 쓰인 태그와 녹음 수. 많이 쓴 순
pub fn vocabulary(recordings: &[Recording], except: &str) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in recordings.iter().filter(|r| r.id != except) {
        // 예전 AI가 붙인 "업무/…" 같은 포괄 분류는 걷어 내고 보여 줘야 AI가 그대로 따라 쓰지 않는다
        for t in r.tags.iter().filter_map(|t| strip_generic(t)) {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn head(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

fn tail(text: &str, n: usize) -> &str {
    let count = text.chars().count();
    if count <= n {
        return text;
    }
    match text.char_indices().nth(count - n) {
        Some((i, _)) => &text[i..],
        None => text,
    }
}

fn excerpt(transcript: &str, has_summary: bool) -> String {
    if has_summary {
        let h = head(transcript, EXCERPT_WITH_SUMMARY);
        return if h.len() < transcript.len() { format!("{h}\n(뒤는 생략)") } else { h.to_string() };
    }
    if transcript.chars().count() <= EXCERPT_HEAD + EXCERPT_TAIL {
        return transcript.to_string();
    }
    format!("{}\n(중간 생략)\n{}", head(transcript, EXCERPT_HEAD), tail(transcript, EXCERPT_TAIL))
}

/// 앱이 붙이는 기본 제목("녹음 2026-10-31 12:00")인지. AI는 이 제목만 바꾼다. 예전에 손으로 바꾼 제목을 지키려는 것이다
pub fn is_default_title(title: &str) -> bool {
    let Some(rest) = title.strip_prefix("녹음 ") else { return false };
    chrono::NaiveDateTime::parse_from_str(rest, "%Y-%m-%d %H:%M").is_ok()
}

/// events: 녹음 시각과 겹치는 캘린더 일정 후보(calendar::events_for). AI가 대화와 맞는지 확인해 하나를 고르거나 고르지 않는다
pub fn build_prompt(
    recording: &Recording,
    transcript: &Transcript,
    summary: Option<&str>,
    vocab: &[(String, usize)],
    events: &[CalendarEvent],
) -> String {
    let vocab_lines = if vocab.is_empty() {
        "아직 없음".to_string()
    } else {
        vocab.iter().take(200).map(|(t, n)| format!("- {t} ({n})")).collect::<Vec<_>>().join("\n")
    };
    let place = recording
        .location
        .as_ref()
        .and_then(|l| l.place.as_deref())
        .map(|p| format!("녹음한 곳: {p}\n"))
        .unwrap_or_default();
    let event = if events.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let local = |t: &chrono::DateTime<chrono::Utc>| t.with_timezone(&chrono::Local).format("%H:%M").to_string();
                let who = if e.attendees.is_empty() { String::new() } else { format!(", 참석자: {}", e.attendees.join(", ")) };
                format!("{}. {} ({}~{}{who})", i + 1, e.title, local(&e.start), local(&e.end))
            })
            .collect();
        format!("녹음 시각과 겹치는 캘린더 일정 후보(시간만 겹친 것이라 이 녹음의 일정인지는 모른다):\n{}\n", lines.join("\n"))
    };
    let summary_block = summary.map(|s| format!("<summary>\n{}\n</summary>\n", s.trim())).unwrap_or_default();
    let text = summarize::format_transcript(recording, transcript);
    format!(
        "녹음 하나를 정리한다. 제목, 모아 볼 태그, 할 일을 고른다.\n\
제목 규칙:\n\
- 30자 이내. 무슨 일로 누구와 나눈 이야기인지 알 수 있게 쓴다. 날짜는 넣지 않는다\n\
- 고른 캘린더 일정이 있으면 그 제목을 참고한다\n\
일정 고르기 규칙:\n\
- 캘린더 일정 후보는 녹음 시각과 겹친다는 것만 안다. 같은 시간에 일정이 여러 개거나, 일정에 가지 않고 다른 일을 했을 수 있다\n\
- 대화에 그 일정의 회사, 행사, 사람, 주제가 분명히 나올 때만 고른다. 대화 내용이 다른 회사나 다른 일이면 고르지 않는다\n\
- 확실하지 않으면 고르지 않는다(null). 고르지 않은 일정의 이름은 제목과 태그에 쓰지 않는다\n\
태그 규칙:\n\
- 1~{MAX_AI_TAGS}개. 녹음을 서로 나눠 모아 볼 수 있는 이름을 고른다: 거래처나 기관, 프로젝트, 주제. 예: HD현대, 데이터센터, 냉각설비\n\
- \"업무\", \"주제\", \"기타\", \"일반\"처럼 거의 모든 녹음에 붙을 포괄 분류는 쓰지 않는다. 앞에 붙이지도 않는다(\"업무/HD현대\"가 아니라 \"HD현대\")\n\
- 계층(/)은 정말 하위 관계일 때만 쓴다. 예: HD현대/슈컴센터. 2단계까지\n\
- 아래 \"쓰던 태그\"에 맞는 것이 있으면 글자 그대로 쓴다. 새로 만들 때는 쓰던 태그의 계층과 이름 짓는 방식을 따른다\n\
- \"회의\", \"녹음\"처럼 모든 녹음에 붙을 말은 쓰지 않는다\n\
할 일 규칙:\n\
- 대화에서 누군가 하기로 분명히 정한 일만 쓴다. 없으면 빈 목록. 최대 {MAX_TODOS}개\n\
- 한 줄로, 누가 할지 알면 앞에 쓴다. 예: \"김대리: 견적서 보내기\"\n\
- 기한을 말했으면 녹음 날짜를 기준으로 YYYY-MM-DD로 바꾼다. 말하지 않았으면 null\n\
공통 규칙:\n\
- 회사, 사람, 제품 이름은 대화나 일정에 분명히 나온 것만 쓴다. 추측하지 않는다\n\
- 대화가 한국어면 한국어로 쓴다\n\
- 답은 JSON 한 줄만 쓴다. 설명을 붙이지 않는다:\n\
{{\"event\": 후보 번호 또는 null, \"title\": \"...\", \"tags\": [\"...\"], \"todos\": [{{\"text\": \"...\", \"due\": \"YYYY-MM-DD\" 또는 null}}]}}\n\n\
쓰던 태그(녹음 수):\n{vocab_lines}\n\n\
지금 제목: {}\n녹음 날짜: {}\n{place}{event}{}\n{summary_block}<transcript>\n{}\n</transcript>\n",
        recording.title,
        recording.created_at.with_timezone(&chrono::Local).format("%Y-%m-%d (%a) %H:%M"),
        summarize::format_notes(recording),
        excerpt(&text, summary.is_some()),
    )
}

#[derive(Deserialize)]
struct RawTodo {
    text: String,
    #[serde(default)]
    due: Option<String>,
}

#[derive(Deserialize)]
struct Answer {
    /// 고른 캘린더 일정 후보 번호(1부터). 숫자가 아니면 고르지 않은 것으로 본다
    #[serde(default)]
    event: Option<serde_json::Value>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    todos: Vec<RawTodo>,
}

/// AI가 정리한 결과
#[derive(Debug, PartialEq)]
pub struct Organized {
    /// 고른 캘린더 일정 후보의 자리(0부터). 범위 확인은 쓰는 쪽에서 한다
    pub event: Option<usize>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    /// 할 일과 기한(YYYY-MM-DD)
    pub todos: Vec<(String, Option<String>)>,
}

/// 거의 모든 녹음에 붙어 모아 보기에 쓸모없는 맨 앞 분류. 0.9.3까지 AI가 "업무/HD현대", "주제/데이터센터"처럼 붙였다
const GENERIC_ROOTS: &[&str] = &["업무", "주제", "기타", "일반", "회의", "미팅", "녹음"];

/// 맨 앞의 포괄 분류를 걷어 낸다. 남는 것이 없으면 None("업무" 하나뿐인 태그). AI가 붙인 태그에만 쓴다(사람이 붙인 태그는 그대로)
/// @see docs/organize.md
pub fn strip_generic(tag: &str) -> Option<String> {
    let parts: Vec<&str> = tag.split('/').collect();
    let start = parts.iter().position(|p| !GENERIC_ROOTS.contains(p))?;
    Some(parts[start..].join("/"))
}

/// AI 답에서 JSON을 찾아 정리한다. 코드 블록이나 앞뒤 설명이 붙어도 읽는다
pub fn parse_answer(answer: &str) -> Result<Organized> {
    let start = answer.find('{').ok_or_else(|| eyre!("AI 답에 JSON이 없습니다: {}", head(answer.trim(), 200)))?;
    let end = answer.rfind('}').filter(|e| *e > start).ok_or_else(|| eyre!("AI 답의 JSON이 닫히지 않았습니다"))?;
    let parsed: Answer = serde_json::from_str(&answer[start..=end]).map_err(|e| eyre!("AI 답을 읽지 못했습니다: {e}"))?;
    let stripped: Vec<String> = sori_sync::tags::normalize_tags(&parsed.tags).iter().filter_map(|t| strip_generic(t)).collect();
    let mut tags = sori_sync::tags::normalize_tags(&stripped);
    tags.truncate(MAX_AI_TAGS);
    let title = parsed
        .title
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().take(60).collect());
    let todos = parsed
        .todos
        .into_iter()
        .filter_map(|t| {
            let text = t.text.split_whitespace().collect::<Vec<_>>().join(" ");
            let due = t.due.filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
            (!text.is_empty()).then_some((text, due))
        })
        .take(MAX_TODOS)
        .collect();
    let event = parsed.event.and_then(|v| v.as_u64()).filter(|n| *n >= 1).map(|n| n as usize - 1);
    Ok(Organized { event, title, tags, todos })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 코드_블록과_설명이_붙은_답에서도_태그를_읽는다() {
        let answer = "다음과 같습니다.\n```json\n{\"tags\": [\" 업무 / 영업 \", \"고객미팅\", \"업무/영업\", \"a\", \"b\"]}\n```";
        let o = parse_answer(answer).unwrap();
        assert_eq!(o.tags, vec!["영업", "고객미팅", "a"]);
        assert_eq!(o.title, None);
        assert!(o.todos.is_empty());
    }

    #[test]
    fn 제목과_할_일을_읽고_잘못된_기한은_버린다() {
        let answer = r#"{"title": " 한빛로보틱스  견적 협의 ", "tags": ["업무/영업"], "todos": [{"text": "김대리: 견적서 보내기", "due": "2026-11-07"}, {"text": "자료 공유", "due": "다음 주"}, {"text": " ", "due": null}]}"#;
        let o = parse_answer(answer).unwrap();
        assert_eq!(o.title.as_deref(), Some("한빛로보틱스 견적 협의"));
        assert_eq!(
            o.todos,
            vec![("김대리: 견적서 보내기".to_string(), Some("2026-11-07".to_string())), ("자료 공유".to_string(), None)]
        );
    }

    #[test]
    fn 고른_일정_번호를_읽고_고르지_않았으면_none() {
        assert_eq!(parse_answer(r#"{"event": 2, "tags": []}"#).unwrap().event, Some(1));
        assert_eq!(parse_answer(r#"{"event": null, "tags": []}"#).unwrap().event, None);
        assert_eq!(parse_answer(r#"{"event": "1", "tags": []}"#).unwrap().event, None);
        assert_eq!(parse_answer(r#"{"event": 0, "tags": []}"#).unwrap().event, None);
        assert_eq!(parse_answer(r#"{"tags": []}"#).unwrap().event, None);
    }

    #[test]
    fn 기본_제목만_ai가_바꾼다() {
        assert!(is_default_title("녹음 2026-10-31 12:00"));
        assert!(!is_default_title("한빛로보틱스 미팅"));
        assert!(!is_default_title("녹음 메모"));
    }

    #[test]
    fn 포괄_분류는_걷어_낸다() {
        assert_eq!(strip_generic("업무/HD현대").as_deref(), Some("HD현대"));
        assert_eq!(strip_generic("업무/주제/데이터센터").as_deref(), Some("데이터센터"));
        assert_eq!(strip_generic("HD현대/슈컴센터").as_deref(), Some("HD현대/슈컴센터"));
        assert_eq!(strip_generic("업무"), None);
        let o = parse_answer(r#"{"tags": ["업무/HD현대", "주제/데이터센터", "업무"]}"#).unwrap();
        assert_eq!(o.tags, vec!["HD현대", "데이터센터"]);
    }

    #[test]
    fn json이_없으면_오류() {
        assert!(parse_answer("태그를 고를 수 없습니다").is_err());
    }

    #[test]
    fn 긴_전사는_앞뒤만_넘긴다() {
        let long = "가".repeat(EXCERPT_HEAD + EXCERPT_TAIL + 10);
        let e = excerpt(&long, false);
        assert!(e.contains("(중간 생략)"));
        assert_eq!(e.chars().filter(|c| *c == '가').count(), EXCERPT_HEAD + EXCERPT_TAIL);
        assert!(excerpt(&long, true).contains("(뒤는 생략)"));
    }

    #[test]
    fn 같은_녹음은_동시에_두_번_분류하지_않는다() {
        let a = begin("x");
        assert!(a.is_some());
        assert!(begin("x").is_none());
        drop(a);
        assert!(begin("x").is_some());
    }
}
