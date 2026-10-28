//! 모든 녹음의 제목, 요약, 전사에서 글자를 찾는다. 빠른 찾기 창(⌘K)이 쓴다.
//! @see docs/shortcuts-and-deep-links.md

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use eyre::Result;
use serde::Serialize;

use crate::engine::Transcript;
use crate::store::{Recording, Store};

/// 녹음 하나에서 보여 줄 최대 결과 수. 나머지는 개수만 알린다
const HITS_PER_RECORDING: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HitKind {
    Summary,
    Utterance,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub kind: HitKind,
    pub start: Option<f32>,
    pub speaker: Option<i32>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingHits {
    pub recording: Recording,
    pub title_match: bool,
    pub hits: Vec<Hit>,
    pub total: usize,
}

/// 전사 파일은 1시간 회의면 수백 KB라, 글자를 칠 때마다 다시 읽지 않도록 수정 시각과 함께 들고 있는다
#[derive(Default)]
pub struct Index {
    transcripts: Mutex<HashMap<String, (SystemTime, Arc<Transcript>)>>,
}

impl Index {
    fn transcript(&self, store: &Store, id: &str) -> Option<Arc<Transcript>> {
        let modified = store.transcript_modified(id)?;
        let mut cache = self.transcripts.lock().unwrap();
        if let Some((at, t)) = cache.get(id) {
            if *at == modified {
                return Some(t.clone());
            }
        }
        let t = Arc::new(store.get_transcript(id).ok()??);
        cache.insert(id.to_string(), (modified, t.clone()));
        Some(t)
    }
}

/// 빈 검색어면 모든 녹음을 최신순으로 돌려준다. 결과가 없는 녹음은 빠진다
pub fn search(store: &Store, index: &Index, query: &str) -> Result<Vec<RecordingHits>> {
    let needle = normalize(query);
    let mut out = Vec::new();
    for recording in store.list()? {
        if needle.is_empty() {
            out.push(RecordingHits { recording, title_match: false, hits: Vec::new(), total: 0 });
            continue;
        }
        let transcript = index.transcript(store, &recording.id);
        let summary = store.get_summary(&recording.id).ok().flatten();
        let mut hits = find_hits(transcript.as_deref(), summary.as_ref().map(|s| s.markdown.as_str()), &needle);
        let title_match = normalize(&recording.title).contains(&needle);
        if !title_match && hits.is_empty() {
            continue;
        }
        let total = hits.len();
        hits.truncate(HITS_PER_RECORDING);
        out.push(RecordingHits { recording, title_match, hits, total });
    }
    Ok(out)
}

/// 대소문자와 띄어쓰기를 무시한다. 한국어는 띄어쓰기를 다르게 쳐도 찾아야 한다
pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

fn find_hits(transcript: Option<&Transcript>, summary: Option<&str>, needle: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    if let Some(markdown) = summary {
        for line in markdown.lines() {
            let text = strip_markdown(line);
            if !text.is_empty() && normalize(&text).contains(needle) {
                hits.push(Hit { kind: HitKind::Summary, start: None, speaker: None, text });
            }
        }
    }
    if let Some(t) = transcript {
        for u in &t.utterances {
            if normalize(&u.text).contains(needle) {
                hits.push(Hit {
                    kind: HitKind::Utterance,
                    start: Some(u.start),
                    speaker: Some(u.speaker),
                    text: u.text.trim().to_string(),
                });
            }
        }
    }
    hits
}

/// 요약 한 줄에서 제목, 목록, 강조 표시를 뗀다
fn strip_markdown(line: &str) -> String {
    let t = line.trim().trim_start_matches(['#', '>', '-', '*', '+', ' ']);
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
    let t = t.strip_prefix(". ").unwrap_or(t);
    t.replace("**", "").replace('`', "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::transcribe::Utterance;

    fn transcript(texts: &[&str]) -> Transcript {
        Transcript {
            duration: 10.0,
            turns: Vec::new(),
            utterances: texts
                .iter()
                .enumerate()
                .map(|(i, t)| Utterance {
                    start: i as f32,
                    end: i as f32 + 1.0,
                    speaker: i as i32 % 2,
                    text: t.to_string(),
                    lang: "ko".into(),
                })
                .collect(),
            origins: Default::default(),
        }
    }

    #[test]
    fn 띄어쓰기와_대소문자를_무시하고_찾는다() {
        let t = transcript(&["다음 주 금요일까지 배포합니다", "See you on Friday"]);

        let hits = find_hits(Some(&t), None, &normalize("금요일 까지"));
        let en = find_hits(Some(&t), None, &normalize("FRIDAY"));

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start, Some(0.0));
        assert_eq!(en[0].speaker, Some(1));
    }

    #[test]
    fn 요약은_서식을_떼고_줄_단위로_찾는다() {
        let summary = "## 결정 사항\n- **배포**는 금요일\n1. 테스트 먼저";

        let hits = find_hits(None, Some(summary), &normalize("배포"));

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "배포는 금요일");
        assert_eq!(hits[0].kind, HitKind::Summary);
    }

    #[test]
    fn 번호_목록의_번호를_뗀다() {
        assert_eq!(strip_markdown("12. 테스트 먼저"), "테스트 먼저");
    }
}
