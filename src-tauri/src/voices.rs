//! 이름 붙인 화자의 목소리 특징을 모아 두고, 새 녹음의 화자와 맞춰 본다.
//! 목소리 특징은 이 컴퓨터의 voices.json에만 있다.
//! @see docs/voices.md

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use eyre::Result;
use serde::{Deserialize, Serialize};

use crate::engine::voiceprint::{normalize, similarity};

/// 이 값 이상 비슷해야 같은 사람으로 본다. TTS 회의 녹음을 앞뒤로 나눠 재니 같은 사람 0.99, 다른 사람 0.18~0.19였다
/// (3D-Speaker ERes2Net, examples/voices_check.rs). 사람 목소리는 마이크와 방이 바뀌면 낮아지므로 여유를 둔다
pub const THRESHOLD: f32 = 0.6;
/// 한 사람에게 모아 두는 녹음 수. 오래된 것부터 버린다
const MAX_SAMPLES: usize = 12;
/// 이보다 비슷한 표본은 같은 녹음을 다시 넣은 것으로 보고 더하지 않는다
const DUPLICATE: f32 = 0.995;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Voice {
    id: String,
    name: String,
    samples: Vec<Vec<f32>>,
    updated_at: DateTime<Utc>,
}

impl Voice {
    fn centroid(&self) -> Vec<f32> {
        let mut sum = vec![0.0f32; self.samples.first().map_or(0, Vec::len)];
        for s in &self.samples {
            sum.iter_mut().zip(s).for_each(|(a, b)| *a += b);
        }
        normalize(sum)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    pub samples: usize,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub speaker: i32,
    pub voice_id: String,
    pub name: String,
    pub score: f32,
}

pub struct VoiceBook {
    path: PathBuf,
    lock: Mutex<()>,
}

impl VoiceBook {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    fn load(&self) -> Vec<Voice> {
        fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, voices: &[Voice]) -> Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(voices)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<VoiceInfo> {
        let _guard = self.lock.lock().unwrap();
        let mut list: Vec<VoiceInfo> = self
            .load()
            .into_iter()
            .map(|v| VoiceInfo { id: v.id, name: v.name, samples: v.samples.len(), updated_at: v.updated_at })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// 이름의 목소리에 표본을 더한다. 처음 보는 이름이면 새로 만든다. 목소리 id를 돌려준다
    pub fn remember(&self, name: &str, embedding: &[f32]) -> Result<String> {
        let _guard = self.lock.lock().unwrap();
        let mut voices = self.load();
        let sample = normalize(embedding.to_vec());
        let id = match voices.iter_mut().find(|v| v.name == name) {
            Some(v) => {
                if !v.samples.iter().any(|s| similarity(s, &sample) > DUPLICATE) {
                    v.samples.push(sample);
                    if v.samples.len() > MAX_SAMPLES {
                        v.samples.remove(0);
                    }
                }
                v.updated_at = Utc::now();
                v.id.clone()
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                voices.push(Voice { id: id.clone(), name: name.to_string(), samples: vec![sample], updated_at: Utc::now() });
                id
            }
        };
        self.save(&voices)?;
        Ok(id)
    }

    /// 잘못 기억한 표본을 지운다. 이름을 바꿨을 때 예전 이름 쪽에서 이 녹음의 목소리를 뺀다
    pub fn forget_sample(&self, voice_id: &str, embedding: &[f32]) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut voices = self.load();
        let sample = normalize(embedding.to_vec());
        if let Some(v) = voices.iter_mut().find(|v| v.id == voice_id) {
            v.samples.retain(|s| similarity(s, &sample) <= DUPLICATE);
        }
        voices.retain(|v| !v.samples.is_empty());
        self.save(&voices)
    }

    pub fn rename(&self, voice_id: &str, name: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut voices = self.load();
        if let Some(v) = voices.iter_mut().find(|v| v.id == voice_id) {
            v.name = name.to_string();
        }
        self.save(&voices)
    }

    pub fn delete(&self, voice_id: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut voices = self.load();
        voices.retain(|v| v.id != voice_id);
        self.save(&voices)
    }

    /// 화자마다 가장 비슷한 목소리를 찾는다. 한 목소리는 한 화자에게만 붙인다
    pub fn match_speakers(&self, speakers: &BTreeMap<i32, Vec<f32>>, skip: &BTreeSet<i32>) -> Vec<Match> {
        let voices = {
            let _guard = self.lock.lock().unwrap();
            self.load()
        };
        assign(&voices.iter().map(|v| (v.id.clone(), v.name.clone(), v.centroid())).collect::<Vec<_>>(), speakers, skip)
    }
}

/// 비슷한 쌍부터 차례로 짝짓는다. 두 화자가 같은 목소리에 가장 가까워도 더 비슷한 쪽만 이름을 받는다
fn assign(voices: &[(String, String, Vec<f32>)], speakers: &BTreeMap<i32, Vec<f32>>, skip: &BTreeSet<i32>) -> Vec<Match> {
    let mut pairs: Vec<Match> = Vec::new();
    for (&speaker, emb) in speakers.iter().filter(|(s, _)| !skip.contains(s)) {
        for (id, name, centroid) in voices {
            let score = similarity(centroid, emb);
            if score >= THRESHOLD {
                pairs.push(Match { speaker, voice_id: id.clone(), name: name.clone(), score });
            }
        }
    }
    pairs.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut used_speakers = BTreeSet::new();
    let mut used_voices = BTreeSet::new();
    pairs
        .into_iter()
        .filter(|m| used_speakers.insert(m.speaker) && used_voices.insert(m.voice_id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vec<f32> {
        normalize(vec![x, y])
    }

    #[test]
    fn 가장_비슷한_쌍부터_한_번씩만_짝짓는다() {
        let voices = vec![("a".into(), "민지".into(), v(1.0, 0.0)), ("b".into(), "준호".into(), v(0.0, 1.0))];
        let speakers = BTreeMap::from([(0, v(0.9, 0.1)), (1, v(0.95, 0.05)), (2, v(0.1, 0.9))]);

        let matches = assign(&voices, &speakers, &BTreeSet::new());

        let named: BTreeMap<i32, &str> = matches.iter().map(|m| (m.speaker, m.name.as_str())).collect();
        assert_eq!(named, BTreeMap::from([(1, "민지"), (2, "준호")]));
    }

    #[test]
    fn 기준보다_덜_비슷하면_이름을_붙이지_않는다() {
        let voices = vec![("a".into(), "민지".into(), v(1.0, 0.0))];
        let speakers = BTreeMap::from([(0, v(0.5, 1.0))]);

        assert!(assign(&voices, &speakers, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn 이미_이름이_있는_화자는_건너뛴다() {
        let voices = vec![("a".into(), "민지".into(), v(1.0, 0.0))];
        let speakers = BTreeMap::from([(0, v(1.0, 0.0))]);

        assert!(assign(&voices, &speakers, &BTreeSet::from([0])).is_empty());
    }

    #[test]
    fn 같은_표본은_두_번_넣지_않고_오래된_표본부터_버린다() {
        let path = std::env::temp_dir().join(format!("sori-voices-{}.json", uuid::Uuid::new_v4()));
        let book = VoiceBook::new(path.clone());

        let one_hot = |i: usize| (0..32).map(|j| if j == i { 1.0 } else { 0.0 }).collect::<Vec<f32>>();
        let id = book.remember("민지", &one_hot(0)).unwrap();
        book.remember("민지", &one_hot(0)).unwrap();
        assert_eq!(book.list()[0].samples, 1);
        for i in 1..=20 {
            book.remember("민지", &one_hot(i)).unwrap();
        }

        let list = book.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].samples, MAX_SAMPLES);
        let _ = fs::remove_file(path);
    }
}
