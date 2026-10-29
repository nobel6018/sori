pub mod audio;
pub mod diarize;
pub mod origin;
pub mod recluster;
pub mod transcribe;
pub mod voiceprint;
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

    let progress = on_progress.clone();
    let progress = move |p| progress(Stage::Diarizing, p);
    let turns = match &options.diarize_worker {
        Some(exe) => worker::diarize_in_child(exe, audio_path, &models.segmentation, &models.embedding, options.num_speakers, &cancel, progress)?,
        None => diarize::diarize(
            &samples,
            &models.segmentation,
            &models.embedding,
            &DiarizeOptions {
                num_speakers: options.num_speakers,
                ..Default::default()
            },
            progress,
        )?,
    };
    // 화자 분리가 말소리를 못 찾았으면 녹음 전체를 한 사람으로 보고 whisper에 맡긴다.
    // whisper가 더 작은 말소리도 받아 적고, 정말 말이 없으면 빈 전사가 된다
    let turns = if turns.is_empty() && duration > 0.0 {
        vec![SpeakerTurn { start: 0.0, end: duration, speaker: 0 }]
    } else {
        turns
    };
    // sherpa의 묶음은 10초 창 안에서 먼저 화자를 정해 비슷한 목소리가 섞이고, 자동 모드는
    // 기준값 하나로 정해 회의 녹음을 수십 명으로 쪼갰다(32분 회의가 100명).
    // 구간마다 목소리 특징을 따로 뽑아 다시 묶는다. 화자 수를 알면 그 수로, 모르면 유사도 기준으로 멈춘다
    check_cancel()?;
    let turns = match options.num_speakers {
        Some(1) => turns.into_iter().map(|t| SpeakerTurn { speaker: 0, ..t }).collect(),
        Some(k) if k >= 2 => recluster::recluster(&turns, &samples, &models.embedding, recluster::Speakers::Count(k as usize))?,
        _ => recluster::recluster(&turns, &samples, &models.embedding, recluster::Speakers::Auto(recluster::AUTO_THRESHOLD))?,
    };

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

    // 트랙 분석은 덤이라 실패해도 전사 결과는 돌려준다
    let origins = match &options.tracks {
        Some((mic, system)) => match (audio::load_mono_16k(mic), audio::load_mono_16k(system)) {
            (Ok(mic), Ok(system)) => origin::classify(&turns, &mic, &system),
            _ => Default::default(),
        },
        None => Default::default(),
    };

    // 목소리 기억도 덤이다. 실패하면 이름 자동 붙이기만 빠진다
    let speaker_embeddings = voiceprint::speaker_embeddings(&turns, &samples, &models.embedding).unwrap_or_default();

    Ok(Transcript {
        duration,
        turns,
        utterances,
        origins,
        speaker_embeddings,
    })
}

/// 목소리 특징 없이 저장된 예전 전사에서 화자별 특징을 새로 뽑는다
pub fn embeddings_for(audio_path: &Path, turns: &[SpeakerTurn], embedding_model: &Path) -> Result<std::collections::BTreeMap<i32, Vec<f32>>> {
    let samples = audio::load_mono_16k(audio_path)?;
    voiceprint::speaker_embeddings(turns, &samples, embedding_model)
}
