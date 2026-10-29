use std::path::Path;

use eyre::Result;
use serde::{Deserialize, Serialize};
use sherpa_rs::diarize::{Diarize, DiarizeConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub start: f32,
    pub end: f32,
    pub speaker: i32,
}

pub struct DiarizeOptions {
    /// None이면 threshold로 화자 수를 추정한다
    pub num_speakers: Option<i32>,
    pub threshold: f32,
}

impl Default for DiarizeOptions {
    fn default() -> Self {
        // 0.5는 90초 통화 녹음에서 20명으로 과분할됐고 0.9가 가장 깔끔했다.
        // 회의실 잔향 녹음은 0.9에서도 과분할되므로 화자 수를 받는 것이 우선이다
        Self {
            num_speakers: None,
            threshold: 0.9,
        }
    }
}

pub fn diarize(
    samples: &[f32],
    segmentation_model: &Path,
    embedding_model: &Path,
    options: &DiarizeOptions,
    on_progress: impl Fn(f32) + Send + 'static,
) -> Result<Vec<SpeakerTurn>> {
    let config = DiarizeConfig {
        num_clusters: Some(options.num_speakers.unwrap_or(-1)),
        threshold: Some(options.threshold),
        min_duration_on: Some(0.3),
        min_duration_off: Some(0.5),
        provider: None,
        debug: false,
    };
    let mut sd = Diarize::new(segmentation_model, embedding_model, config)?;

    // sherpa-onnx는 이 콜백의 반환값을 쓰지 않아 화자 분리 도중에는 멈출 수 없다. 취소는 이 단계가 끝난 직후 적용된다
    let callback = Box::new(move |done: i32, total: i32| {
        if total > 0 {
            on_progress(done as f32 / total as f32);
        }
        0
    });
    // 화자 분리 모델이 말소리를 하나도 못 찾으면 sherpa-rs는 결과 없음을 오류로 돌려준다.
    // 작게 말했거나 아주 짧은 녹음에서 생기므로, 빈 결과로 바꿔 받아 적기는 계속하게 한다
    let segments = match sd.compute(samples.to_vec(), Some(callback)) {
        Ok(s) => s,
        Err(e) if e.to_string().contains("No segments found") => Vec::new(),
        Err(e) => return Err(e),
    };

    Ok(segments
        .into_iter()
        .map(|s| SpeakerTurn {
            start: s.start,
            end: s.end,
            speaker: s.speaker,
        })
        .collect())
}
