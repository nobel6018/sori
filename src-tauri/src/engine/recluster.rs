use std::path::Path;

use eyre::Result;
use sherpa_rs::speaker_id::{EmbeddingExtractor, ExtractorConfig};

use super::audio::SAMPLE_RATE;
use super::diarize::SpeakerTurn;

/// 이보다 긴 구간만 묶음을 만드는 데 쓴다. 짧은 구간의 임베딩은 흔들린다
const CLUSTER_MIN_SECS: f32 = 1.0;
/// 이보다 짧은 구간은 임베딩을 뽑지 않고 시간상 가장 가까운 구간을 따른다
const EMBED_MIN_SECS: f32 = 0.5;
/// 긴 구간은 가운데 이만큼만 쓴다. 1시간 회의에서도 몇 초 안에 끝나게 한다
const EMBED_MAX_SECS: f32 = 15.0;

/// sherpa-onnx는 10초 창 안에서 "창 속 화자"를 먼저 정한 뒤 그 묶음의 임베딩으로 군집화한다.
/// 번갈아 말하는 비슷한 목소리 둘이 창 안에서 한 묶음이 되면 임베딩이 둘의 중간값이 되어,
/// 화자 수를 지정해도 다시 나뉘지 않는다(3개 언어 TTS 회의에서 두 여성 목소리가 이렇게 합쳐졌다).
/// 구간 경계는 sherpa 결과를 그대로 쓰고, 구간마다 임베딩을 따로 뽑아 지정한 화자 수로 다시 묶는다
/// 몇 명으로 묶을지
#[derive(Debug, Clone, Copy)]
pub enum Speakers {
    /// 사용자가 알려 준 화자 수
    Count(usize),
    /// 평균 유사도가 이 값 아래로 떨어질 때까지 묶는다. 말한 시간이 아주 짧은 묶음은 가까운 화자에 붙인다
    Auto(f32),
}

/// 자동 추정 기준값. 화자 수를 아는 녹음 6개로 맞췄다(examples/speaker_count_check.rs, docs/speaker-count.md)
pub const AUTO_THRESHOLD: f32 = 0.45;
/// 전체 말한 시간에서 이보다 적게 말한 묶음은 따로 세지 않는다. 기침, 잡음, 짧은 맞장구가 화자로 잡히는 것을 막는다
const MIN_SPEAKER_SHARE: f32 = 0.02;
const MIN_SPEAKER_SECS: f32 = 8.0;

pub fn recluster(turns: &[SpeakerTurn], samples: &[f32], embedding_model: &Path, speakers: Speakers) -> Result<Vec<SpeakerTurn>> {
    let embeddings = turn_embeddings(turns, samples, embedding_model)?;
    let anchors: Vec<usize> = (0..turns.len())
        .filter(|&i| embeddings[i].is_some() && turns[i].end - turns[i].start >= CLUSTER_MIN_SECS)
        .collect();
    let min_needed = match speakers {
        Speakers::Count(k) => k,
        Speakers::Auto(_) => 2,
    };
    if anchors.len() < min_needed {
        return Ok(turns.to_vec());
    }
    let anchor_vecs: Vec<&[f32]> = anchors.iter().map(|&i| embeddings[i].as_deref().unwrap()).collect();
    let durations: Vec<f32> = anchors.iter().map(|&i| turns[i].end - turns[i].start).collect();
    let anchor_labels = match speakers {
        Speakers::Count(k) => cluster(&anchor_vecs, Stop::Count(k)),
        Speakers::Auto(threshold) => drop_minor(&anchor_vecs, &durations, cluster(&anchor_vecs, Stop::Below(threshold))),
    };
    let k = anchor_labels.iter().max().map_or(1, |m| m + 1);
    label_turns(turns, &embeddings, &anchors, &anchor_vecs, &anchor_labels, k)
}

/// 구간마다 목소리 특징. 0.5초보다 짧은 구간은 None
pub fn turn_embeddings(turns: &[SpeakerTurn], samples: &[f32], embedding_model: &Path) -> Result<Vec<Option<Vec<f32>>>> {
    let mut extractor = EmbeddingExtractor::new(ExtractorConfig {
        model: embedding_model.to_string_lossy().into_owned(),
        num_threads: Some(std::thread::available_parallelism().map_or(2, |n| n.get())),
        ..Default::default()
    })?;

    turns
        .iter()
        .map(|t| {
            let dur = t.end - t.start;
            if dur < EMBED_MIN_SECS {
                return Ok(None);
            }
            let mid = (t.start + t.end) / 2.0;
            let half = dur.min(EMBED_MAX_SECS) / 2.0;
            let from = (((mid - half) * SAMPLE_RATE as f32) as usize).min(samples.len());
            let to = (((mid + half) * SAMPLE_RATE as f32) as usize).min(samples.len());
            let v = extractor.compute_speaker_embedding(samples[from..to].to_vec(), SAMPLE_RATE)?;
            Ok(Some(normalize(v)))
        })
        .collect()
}

/// 묶음 결과로 모든 구간에 화자 번호를 붙이고, 먼저 말한 사람부터 번호를 다시 매긴다
fn label_turns(
    turns: &[SpeakerTurn],
    embeddings: &[Option<Vec<f32>>],
    anchors: &[usize],
    anchor_vecs: &[&[f32]],
    anchor_labels: &[usize],
    k: usize,
) -> Result<Vec<SpeakerTurn>> {

    let mut centroids = vec![vec![0.0f32; anchor_vecs[0].len()]; k];
    for (v, &label) in anchor_vecs.iter().zip(anchor_labels) {
        centroids[label].iter_mut().zip(v.iter()).for_each(|(c, x)| *c += x);
    }
    let centroids: Vec<Vec<f32>> = centroids.into_iter().map(normalize).collect();

    let mut labels: Vec<Option<usize>> = vec![None; turns.len()];
    for (&i, &label) in anchors.iter().zip(anchor_labels) {
        labels[i] = Some(label);
    }
    // 묶음에 안 쓴 0.5~1초 구간은 가장 비슷한 묶음 중심에 붙인다
    for i in 0..turns.len() {
        if labels[i].is_none() {
            if let Some(v) = &embeddings[i] {
                labels[i] = (0..k).max_by(|&a, &b| dot(v, &centroids[a]).total_cmp(&dot(v, &centroids[b])));
            }
        }
    }
    // 0.5초 미만 구간은 시간상 가장 가까운 구간의 화자를 따른다
    let known: Vec<usize> = (0..turns.len()).filter(|&i| labels[i].is_some()).collect();
    for i in 0..turns.len() {
        if labels[i].is_none() {
            let nearest = known.iter().min_by(|&&a, &&b| gap(&turns[i], &turns[a]).total_cmp(&gap(&turns[i], &turns[b])));
            labels[i] = nearest.and_then(|&j| labels[j]);
        }
    }

    // 먼저 말한 사람이 화자 1이 되도록 번호를 다시 매긴다
    let mut order: Vec<usize> = Vec::new();
    for l in labels.iter().flatten() {
        if !order.contains(l) {
            order.push(*l);
        }
    }
    Ok(turns
        .iter()
        .zip(&labels)
        .map(|(t, l)| SpeakerTurn {
            speaker: l.and_then(|l| order.iter().position(|&o| o == l)).unwrap_or(0) as i32,
            ..t.clone()
        })
        .collect())
}

#[derive(Debug, Clone, Copy)]
pub enum Stop {
    Count(usize),
    Below(f32),
}

/// 평균 연결 군집화. 평균 코사인 유사도가 가장 높은 두 묶음을 합치기를, 묶음이 k개가 되거나
/// 가장 비슷한 두 묶음의 유사도가 기준 아래가 될 때까지 되풀이한다. 묶음 번호(0부터)를 돌려준다
pub fn cluster(vecs: &[&[f32]], stop: Stop) -> Vec<usize> {
    let n = vecs.len();
    let sim: Vec<Vec<f32>> = (0..n).map(|i| (0..n).map(|j| dot(vecs[i], vecs[j])).collect()).collect();
    let mut groups: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let k = match stop {
        Stop::Count(k) => k.max(1),
        Stop::Below(_) => 1,
    };
    while groups.len() > k {
        let mut best = (0, 1, f32::MIN);
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                let total: f32 = groups[a].iter().flat_map(|&i| groups[b].iter().map(move |&j| (i, j))).map(|(i, j)| sim[i][j]).sum();
                let avg = total / (groups[a].len() * groups[b].len()) as f32;
                if avg > best.2 {
                    best = (a, b, avg);
                }
            }
        }
        if let Stop::Below(threshold) = stop {
            if best.2 < threshold {
                break;
            }
        }
        let merged = groups.remove(best.1);
        groups[best.0].extend(merged);
    }
    let mut labels = vec![0; n];
    for (label, g) in groups.iter().enumerate() {
        for &i in g {
            labels[i] = label;
        }
    }
    labels
}

/// 말한 시간이 아주 짧은 묶음은 가장 비슷한 큰 묶음에 붙이고, 번호를 0부터 다시 매긴다
pub fn drop_minor(vecs: &[&[f32]], durations: &[f32], labels: Vec<usize>) -> Vec<usize> {
    let k = labels.iter().max().map_or(0, |m| m + 1);
    let total: f32 = durations.iter().sum();
    let mut spoken = vec![0.0f32; k];
    for (l, d) in labels.iter().zip(durations) {
        spoken[*l] += d;
    }
    let min = (total * MIN_SPEAKER_SHARE).max(MIN_SPEAKER_SECS);
    let major: Vec<usize> = (0..k).filter(|&c| spoken[c] >= min).collect();
    if major.is_empty() {
        return vec![0; labels.len()];
    }
    let dim = vecs.first().map_or(0, |v| v.len());
    let centroids: Vec<Vec<f32>> = major
        .iter()
        .map(|&c| {
            let mut sum = vec![0.0f32; dim];
            for (v, l) in vecs.iter().zip(&labels) {
                if *l == c {
                    sum.iter_mut().zip(v.iter()).for_each(|(a, b)| *a += b);
                }
            }
            normalize(sum)
        })
        .collect();
    labels
        .iter()
        .zip(vecs)
        .map(|(l, v)| match major.iter().position(|m| m == l) {
            Some(i) => i,
            None => (0..major.len()).max_by(|&a, &b| dot(v, &centroids[a]).total_cmp(&dot(v, &centroids[b]))).unwrap(),
        })
        .collect()
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(f32::EPSILON);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn gap(a: &SpeakerTurn, b: &SpeakerTurn) -> f32 {
    (a.start - b.end).max(b.start - a.end).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 비슷한_벡터끼리_k개로_묶는다() {
        let a1 = normalize(vec![1.0, 0.1, 0.0]);
        let a2 = normalize(vec![0.9, 0.2, 0.0]);
        let b1 = normalize(vec![0.0, 1.0, 0.1]);
        let c1 = normalize(vec![0.0, 0.1, 1.0]);
        let c2 = normalize(vec![0.1, 0.0, 0.9]);

        let labels = cluster(&[&a1, &b1, &a2, &c1, &c2], Stop::Count(3));

        assert_eq!(labels[0], labels[2]);
        assert_eq!(labels[3], labels[4]);
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[0], labels[3]);
        assert_ne!(labels[1], labels[3]);
    }
}
