//! 녹음에 질문하기. 답에 근거가 된 시각을 [12:34] 또는 [R3 12:34]로 달게 해, 화면에서 눌러 그 구간을 재생한다.
//! @see docs/ask.md

use serde::Serialize;

use crate::engine::Transcript;
use crate::search::normalize;
use crate::store::Recording;
use crate::summarize::{format_notes, format_transcript, Summary};

/// 전체 녹음에 물을 때 CLI에 넘기는 글자 수 상한. 넘으면 오래된 녹음부터 뺀다
const MAX_CONTEXT_CHARS: usize = 150_000;
/// 전체 녹음에 물을 때 녹음마다 넣는 요약 길이
const SUMMARY_CHARS: usize = 1_500;
/// 요약이 없는 녹음은 전사 앞부분을 이만큼 넣는다
const HEAD_CHARS: usize = 800;
/// 질문 낱말이 들어간 발화를 이만큼까지 앞뒤 발화와 함께 넣는다
const MAX_HITS: usize = 60;

/// 전체 녹음 질문에서 [R3 12:34]의 R3가 어느 녹음인지
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub key: String,
    pub recording_id: String,
    pub title: String,
}

const RULES: &str = "\
아래 기록만 근거로 질문에 한국어로 답하라. 기록은 자동으로 받아 적은 것이라 오류가 있을 수 있다.
- 기록에 없으면 모른다고 답하고 지어내지 마라
- 근거가 된 발화마다 문장 끝에 시각을 대괄호로 달아라. 형식은 아래 지시를 정확히 따른다
- 마크다운으로 짧고 분명하게 쓴다";

pub fn single_prompt(question: &str, recording: &Recording, transcript: &Transcript, summary: Option<&Summary>) -> String {
    let summary = summary.map(|s| format!("\n<summary>\n{}\n</summary>\n", s.markdown)).unwrap_or_default();
    format!(
        "{RULES}\n- 시각 형식: [12:34] (기록에 적힌 그대로)\n\n녹음 제목: {}\n{}{summary}\n<transcript>\n{}\n</transcript>\n\n질문: {question}\n",
        recording.title,
        format_notes(recording),
        format_transcript(recording, transcript),
    )
}

pub struct Candidate<'a> {
    pub recording: &'a Recording,
    pub transcript: Option<&'a Transcript>,
    pub summary: Option<&'a Summary>,
}

/// 녹음이 많으면 전사를 다 넣을 수 없다. 녹음마다 요약을 넣고, 질문 낱말이 들어간 발화를 찾아 앞뒤와 함께 넣는다
pub fn all_prompt(question: &str, candidates: &[Candidate]) -> (String, Vec<SourceRef>) {
    let words = keywords(question);
    let mut sources = Vec::new();
    let mut body = String::new();
    let mut hits_left = MAX_HITS;
    for (i, c) in candidates.iter().enumerate() {
        let key = format!("R{}", i + 1);
        let mut part = format!(
            "<recording id=\"{key}\" title=\"{}\" date=\"{}\">\n",
            c.recording.title,
            c.recording.created_at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
        );
        match (c.summary, c.transcript) {
            (Some(s), _) => part.push_str(&format!("요약:\n{}\n", truncate(&s.markdown, SUMMARY_CHARS))),
            (None, Some(t)) => part.push_str(&format!("전사 앞부분:\n{}\n", truncate(&format_transcript(c.recording, t), HEAD_CHARS))),
            (None, None) => continue,
        }
        if let Some(t) = c.transcript {
            let lines = matching_lines(c.recording, t, &words, &mut hits_left);
            if !lines.is_empty() {
                part.push_str("질문과 관련 있어 보이는 발화:\n");
                part.push_str(&lines.join("\n"));
                part.push('\n');
            }
        }
        part.push_str("</recording>\n");
        if body.len() + part.len() > MAX_CONTEXT_CHARS {
            break;
        }
        body.push_str(&part);
        sources.push(SourceRef { key, recording_id: c.recording.id.clone(), title: c.recording.title.clone() });
    }
    let prompt = format!(
        "{RULES}\n- 시각 형식: [R3 12:34] (녹음 id와 기록에 적힌 시각). 요약만 근거면 [R3]만 단다\n\n<recordings>\n{body}</recordings>\n\n질문: {question}\n"
    );
    (prompt, sources)
}

/// 질문에서 찾을 낱말. 두 글자 이상만 쓰고, 한국어 조사가 붙은 낱말은 끝 글자를 뗀 것도 찾는다("배포는" → "배포")
fn keywords(question: &str) -> Vec<String> {
    // 어느 발화에나 들어갈 만한 질문 낱말은 찾지 않는다
    const STOP: [&str; 16] = ["언제", "어디", "누가", "무엇", "뭐라고", "했지", "했나", "했어", "어떻게", "얼마나", "있었", "있었지", "what", "when", "who", "the"];
    let mut out = Vec::new();
    for w in question.split(|c: char| !c.is_alphanumeric()) {
        let w = normalize(w);
        let n = w.chars().count();
        if n < 2 || STOP.contains(&w.as_str()) {
            continue;
        }
        if n >= 3 && w.chars().last().is_some_and(|c| ('가'..='힣').contains(&c)) {
            out.push(w.chars().take(n - 1).collect());
        }
        out.push(w);
    }
    out.sort();
    out.dedup();
    out
}

fn matching_lines(recording: &Recording, t: &Transcript, words: &[String], left: &mut usize) -> Vec<String> {
    if words.is_empty() {
        return Vec::new();
    }
    let us = &t.utterances;
    let mut picked = std::collections::BTreeSet::new();
    for (i, u) in us.iter().enumerate() {
        if *left == 0 {
            break;
        }
        let text = normalize(&u.text);
        if words.iter().any(|w| text.contains(w.as_str())) {
            *left -= 1;
            picked.extend(i.saturating_sub(1)..=(i + 1).min(us.len() - 1));
        }
    }
    picked
        .into_iter()
        .map(|i| {
            let u = &us[i];
            let name = recording.speaker_names.get(&u.speaker.to_string()).cloned().unwrap_or_else(|| format!("화자 {}", u.speaker + 1));
            let secs = u.start as u32;
            format!("[{:02}:{:02}] {name}: {}", secs / 60, secs % 60, u.text.trim())
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 조사를_뗀_낱말도_찾는다() {
        let words = keywords("배포는 언제 하기로 했지?");

        assert!(words.contains(&"배포".to_string()));
        assert!(words.contains(&"배포는".to_string()));
        assert!(!words.contains(&"언제".to_string()));
    }
}
