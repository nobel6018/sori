pub mod audio;
pub mod diarize;
pub mod origin;
pub mod recluster;
pub mod transcribe;

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
}

pub struct ProcessOptions {
    pub num_speakers: Option<i32>,
    pub languages: Vec<String>,
    /// (마이크 트랙, 시스템 소리 트랙). 앱에서 둘 다 녹음했을 때만 있다
    pub tracks: Option<(PathBuf, PathBuf)>,
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
    let turns = diarize::diarize(
        &samples,
        &models.segmentation,
        &models.embedding,
        &DiarizeOptions {
            num_speakers: options.num_speakers,
            ..Default::default()
        },
        move |p| progress(Stage::Diarizing, p),
    )?;
    // 화자 수를 알면 구간마다 임베딩을 다시 뽑아 그 수로 묶는다. 자동 모드는 기준값이
    // 녹음 환경마다 크게 흔들려(회의실 녹음이 65명으로 쪼개졌다) sherpa 결과를 그대로 둔다
    check_cancel()?;
    let turns = match options.num_speakers {
        Some(k) if k >= 2 => recluster::recluster(&turns, &samples, &models.embedding, k as usize)?,
        _ => turns,
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

    Ok(Transcript {
        duration,
        turns,
        utterances,
        origins,
    })
}
