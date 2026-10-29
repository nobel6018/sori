//! 화자마다 목소리 특징(임베딩) 하나를 만든다. 다음 녹음에서 같은 사람을 알아보는 데 쓴다.
//! @see docs/voices.md

use std::collections::BTreeMap;
use std::path::Path;

use eyre::Result;
use sherpa_rs::speaker_id::{EmbeddingExtractor, ExtractorConfig};

use super::audio::SAMPLE_RATE;
use super::diarize::SpeakerTurn;

/// 이보다 짧은 구간은 목소리 특징이 흔들려 쓰지 않는다
const MIN_SECS: f32 = 1.5;
/// 화자마다 가장 긴 구간 몇 개만 쓴다. 1시간 회의에서도 몇 초 안에 끝난다
const TURNS_PER_SPEAKER: usize = 8;
/// 긴 구간은 가운데 이만큼만 쓴다
const MAX_SECS: f32 = 10.0;

pub fn speaker_embeddings(turns: &[SpeakerTurn], samples: &[f32], embedding_model: &Path) -> Result<BTreeMap<i32, Vec<f32>>> {
    let mut extractor = EmbeddingExtractor::new(ExtractorConfig {
        model: embedding_model.to_string_lossy().into_owned(),
        num_threads: Some(std::thread::available_parallelism().map_or(2, |n| n.get())),
        ..Default::default()
    })?;
    let mut by_speaker: BTreeMap<i32, Vec<&SpeakerTurn>> = BTreeMap::new();
    for t in turns.iter().filter(|t| t.end - t.start >= MIN_SECS) {
        by_speaker.entry(t.speaker).or_default().push(t);
    }
    let mut out = BTreeMap::new();
    for (speaker, mut list) in by_speaker {
        list.sort_by(|a, b| (b.end - b.start).total_cmp(&(a.end - a.start)));
        let mut sum: Option<Vec<f32>> = None;
        for t in list.into_iter().take(TURNS_PER_SPEAKER) {
            let mid = (t.start + t.end) / 2.0;
            let half = (t.end - t.start).min(MAX_SECS) / 2.0;
            let from = (((mid - half) * SAMPLE_RATE as f32) as usize).min(samples.len());
            let to = (((mid + half) * SAMPLE_RATE as f32) as usize).min(samples.len());
            let v = normalize(extractor.compute_speaker_embedding(samples[from..to].to_vec(), SAMPLE_RATE)?);
            match sum.as_mut() {
                Some(s) => s.iter_mut().zip(&v).for_each(|(a, b)| *a += b),
                None => sum = Some(v),
            }
        }
        if let Some(s) = sum {
            out.insert(speaker, normalize(s));
        }
    }
    Ok(out)
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// 두 단위 벡터의 코사인 유사도
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
