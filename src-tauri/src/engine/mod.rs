pub mod audio;
pub mod diarize;
pub mod origin;
pub mod recluster;
pub mod transcribe;
pub mod voiceprint;
pub mod split;
pub mod worker;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use eyre::Result;
use serde::{Deserialize, Serialize};
use whisper_rs::{WhisperContext, WhisperContextParameters};

use diarize::{DiarizeOptions, SpeakerTurn};
use transcribe::{TranscribeOptions, Utterance};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPaths {
    pub whisper: PathBuf,
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// 앞 녹음의 처리가 끝나길 기다린다
    Waiting,
    Decoding,
    Diarizing,
    Transcribing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub duration: f32,
    pub turns: Vec<SpeakerTurn>,
    pub utterances: Vec<Utterance>,
    /// 마이크와 시스템 소리를 따로 녹음한 경우에만 채운다. 판단하지 못한 화자는 빠진다
    #[serde(default)]
    pub origins: std::collections::BTreeMap<i32, origin::Origin>,
    /// 화자 번호 → 목소리 특징(단위 벡터). 화자 이름을 기억하고 다음 녹음에서 알아보는 데 쓴다
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub speaker_embeddings: std::collections::BTreeMap<i32, Vec<f32>>,
}

pub struct ProcessOptions {
    pub num_speakers: Option<i32>,
    pub languages: Vec<String>,
    /// (마이크 트랙, 시스템 소리 트랙). 앱에서 둘 다 녹음했을 때만 있다
    pub tracks: Option<(PathBuf, PathBuf)>,
    /// 있으면 화자 분리를 이 실행 파일의 작업 프로세스로 돌려, 취소하면 바로 멈추게 한다(worker.rs)
    pub diarize_worker: Option<PathBuf>,
    /// false면 화자를 나누지 않고 받아 적기만 한다. 혼자 말한 녹음, 강의에서 더 빠르다
    pub diarize: bool,
}

pub fn load_whisper(path: &Path) -> Result<WhisperContext> {
    let path = path.to_str().ok_or_else(|| eyre::eyre!("모델 경로에 UTF-8이 아닌 문자가 있습니다"))?;
    Ok(WhisperContext::new_with_params(path, WhisperContextParameters::default())?)
}

/// 디코딩, 화자 분리, 전사를 순서대로 돌린다.
/// 화자 분리를 파일 전체에 한 번 먼저 돌려야 조각 사이에서 화자 번호가 일관된다
pub fn process(
    audio_path: &Path,
    whisper: &WhisperContext,
    models: &ModelPaths,
    options: &ProcessOptions,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(Stage, f32) + Send + Sync + Clone + 'static,
) -> Result<Transcript> {
    let check_cancel = || -> Result<()> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            eyre::bail!("취소했습니다");
        }
        Ok(())
    };
    on_progress(Stage::Decoding, 0.0);
    let samples = audio::load_mono_16k(audio_path)?;
    check_cancel()?;
    let duration = samples.len() as f32 / audio::SAMPLE_RATE as f32;

    let speakers = if options.diarize {
        let progress = on_progress.clone();
        let progress = move |p| progress(Stage::Diarizing, p);
        match &options.diarize_worker {
            Some(exe) => worker::speakers_in_child(exe, audio_path, models, options.num_speakers, &cancel, progress)?,
            None => find_speakers(&samples, models, options.num_speakers, progress)?,
        }
    } else {
        // 전사만: 화자를 나누지 않고 조용한 곳에서 잘라 모두 화자 한 명으로 받아 적는다.
        // 여러 사람 목소리가 한 화자로 섞여 있어 목소리 특징은 기억하지 않는다
        Speakers { turns: split::split_by_silence(&samples), speaker_embeddings: Default::default() }
    };
    let Speakers { turns, speaker_embeddings } = speakers;

    check_cancel()?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() as i32);
    let utterances = transcribe::transcribe(
        whisper,
        &samples,
        &turns,
        &TranscribeOptions {
            languages: options.languages.clone(),
            threads,
        },
        cancel,
        |p| on_progress(Stage::Transcribing, p),
    )?;
    let utterances = drop_repeats(utterances);

    // 트랙 분석은 덤이라 실패해도 전사 결과는 돌려준다
    // 화자를 나누지 않았으면 한 화자에 마이크와 원격 목소리가 섞여 있어 가르지 않는다
    let origins = match options.tracks.as_ref().filter(|_| options.diarize) {
        Some((mic, system)) => match (audio::load_mono_16k(mic), audio::load_mono_16k(system)) {
            (Ok(mic), Ok(system)) => origin::classify(&turns, &mic, &system),
            _ => Default::default(),
        },
        None => Default::default(),
    };


    Ok(Transcript {
        duration,
        turns,
        utterances,
        origins,
        speaker_embeddings,
    })
}

/// 같은 화자가 바로 앞과 똑같은 문장을 말한 것으로 나오면 지운다. whisper가 조각 경계에서
/// 앞 문장을 한 번 더 내는 경우가 있었다(전사만 모드, 3개 언어 시험 녹음 104.7초)
fn drop_repeats(utterances: Vec<transcribe::Utterance>) -> Vec<transcribe::Utterance> {
    let mut out: Vec<transcribe::Utterance> = Vec::with_capacity(utterances.len());
    for u in utterances {
        let repeat = out.last().is_some_and(|p| p.speaker == u.speaker && p.text.trim() == u.text.trim() && u.start - p.end < 2.0);
        if !repeat {
            out.push(u);
        }
    }
    out
}

/// 화자 찾기 결과: 구간별 화자와 화자별 목소리 특징
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Speakers {
    pub turns: Vec<SpeakerTurn>,
    pub speaker_embeddings: std::collections::BTreeMap<i32, Vec<f32>>,
}

/// 화자 분리, 다시 묶기, 화자별 목소리 특징. onnxruntime을 쓰는 일은 모두 여기 모았다.
/// 앱은 이 함수를 작업 프로세스(worker.rs)에서 돌려, onnxruntime 예외가 앱을 죽이지 않게 한다
pub fn find_speakers(samples: &[f32], models: &ModelPaths, num_speakers: Option<i32>, on_progress: impl Fn(f32) + Send + 'static) -> Result<Speakers> {
    let duration = samples.len() as f32 / audio::SAMPLE_RATE as f32;
    // sherpa에는 화자 수를 넘기지 않는다. 구간 경계만 쓰고 묶기는 아래에서 다시 하기 때문이다.
    // sherpa-onnx의 군집 개수 지정(num_clusters)은 실제 회의 녹음 5개 모두에서 1명이면 메모리 접근 오류로
    // 죽었고(examples/diarize_check.rs), 0.4.1의 비정상 종료도 화자 수를 넣어 처리하던 중에 났다
    let turns = diarize::diarize(samples, &models.segmentation, &models.embedding, &DiarizeOptions::default(), on_progress)?;
    // 화자 분리가 말소리를 못 찾았으면(작게 말했거나 아주 짧은 녹음) whisper에 맡긴다.
    // whisper가 더 작은 말소리도 받아 적고, 정말 말이 없으면 빈 전사가 된다
    if turns.is_empty() && duration > 0.0 {
        return Ok(Speakers { turns: split::split_by_silence(samples), speaker_embeddings: Default::default() });
    }
    // sherpa의 묶음은 10초 창 안에서 먼저 화자를 정해 비슷한 목소리가 섞이고, 자동 모드는
    // 기준값 하나로 정해 회의 녹음을 수십 명으로 쪼갰다(32분 회의가 100명).
    // 구간마다 목소리 특징을 따로 뽑아 다시 묶는다. 화자 수를 알면 그 수로, 모르면 유사도 기준으로 멈춘다
    let turns = match num_speakers {
        Some(1) => turns.into_iter().map(|t| SpeakerTurn { speaker: 0, ..t }).collect(),
        Some(k) if k >= 2 => recluster::recluster(&turns, samples, &models.embedding, recluster::Speakers::Count(k as usize))?,
        _ => recluster::recluster(&turns, samples, &models.embedding, recluster::Speakers::Auto(recluster::AUTO_THRESHOLD))?,
    };
    // 목소리 기억은 덤이다. 실패하면 이름 자동 붙이기만 빠진다
    let speaker_embeddings = voiceprint::speaker_embeddings(&turns, samples, &models.embedding).unwrap_or_default();
    Ok(Speakers { turns, speaker_embeddings })
}

/// 목소리 특징 없이 저장된 예전 전사에서 화자별 특징을 새로 뽑는다. worker가 있으면 작업 프로세스에서 한다
pub fn embeddings_for(
    audio_path: &Path,
    turns: &[SpeakerTurn],
    embedding_model: &Path,
    worker: Option<&Path>,
) -> Result<std::collections::BTreeMap<i32, Vec<f32>>> {
    if let Some(exe) = worker {
        return worker::embeddings_in_child(exe, audio_path, embedding_model, turns);
    }
    let samples = audio::load_mono_16k(audio_path)?;
    voiceprint::speaker_embeddings(turns, &samples, embedding_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(start: f32, speaker: i32, text: &str) -> transcribe::Utterance {
        transcribe::Utterance { start, end: start + 1.0, speaker, text: text.into(), lang: "ko".into() }
    }

    #[test]
    fn 바로_이어지는_같은_문장만_지운다() {
        let out = drop_repeats(vec![u(0.0, 0, "네"), u(1.2, 0, "네 "), u(10.0, 0, "네"), u(11.0, 1, "네")]);

        assert_eq!(out.iter().map(|x| x.start).collect::<Vec<_>>(), vec![0.0, 10.0, 11.0]);
    }
}
