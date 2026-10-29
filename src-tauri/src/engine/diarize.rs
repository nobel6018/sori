use std::ffi::CString;
use std::path::Path;

use eyre::{bail, Result};
use serde::{Deserialize, Serialize};

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

/// sherpa-onnx 화자 분리. sherpa-rs의 Diarize는 두 모델을 스레드 1개로 고정해(sherpa-rs 0.6.8 diarize.rs)
/// 15분 녹음에 5분 가까이 걸렸다. C API를 직접 불러 코어 수만큼 쓴다
pub fn diarize(
    samples: &[f32],
    segmentation_model: &Path,
    embedding_model: &Path,
    options: &DiarizeOptions,
    on_progress: impl Fn(f32) + Send + 'static,
) -> Result<Vec<SpeakerTurn>> {
    use sherpa_rs_sys as sys;

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()) as i32;
    let segmentation = CString::new(segmentation_model.to_string_lossy().as_bytes())?;
    let embedding = CString::new(embedding_model.to_string_lossy().as_bytes())?;
    let provider = CString::new("cpu")?;
    let config = sys::SherpaOnnxOfflineSpeakerDiarizationConfig {
        segmentation: sys::SherpaOnnxOfflineSpeakerSegmentationModelConfig {
            pyannote: sys::SherpaOnnxOfflineSpeakerSegmentationPyannoteModelConfig { model: segmentation.as_ptr() },
            num_threads: threads,
            debug: 0,
            provider: provider.as_ptr(),
        },
        embedding: sys::SherpaOnnxSpeakerEmbeddingExtractorConfig {
            model: embedding.as_ptr(),
            num_threads: threads,
            debug: 0,
            provider: provider.as_ptr(),
        },
        clustering: sys::SherpaOnnxFastClusteringConfig {
            num_clusters: options.num_speakers.unwrap_or(-1),
            threshold: options.threshold,
        },
        min_duration_on: 0.3,
        min_duration_off: 0.5,
    };

    struct Handle(*const sys::SherpaOnnxOfflineSpeakerDiarization);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { sys::SherpaOnnxDestroyOfflineSpeakerDiarization(self.0) };
        }
    }
    let sd = Handle(unsafe { sys::SherpaOnnxCreateOfflineSpeakerDiarization(&config) });
    if sd.0.is_null() {
        bail!("화자 분리 모델을 불러오지 못했습니다");
    }

    // sherpa-onnx는 이 콜백의 반환값을 쓰지 않아 화자 분리 도중에는 멈출 수 없다.
    // 그래서 앱은 화자 분리를 별도 프로세스로 돌리고, 취소하면 그 프로세스를 끝낸다(src/diarize_worker.rs)
    let mut callback: Box<dyn Fn(f32)> = Box::new(on_progress);
    unsafe extern "C" fn trampoline(done: i32, total: i32, arg: *mut std::ffi::c_void) -> i32 {
        let cb = unsafe { &*(arg as *const Box<dyn Fn(f32)>) };
        if total > 0 {
            cb(done as f32 / total as f32);
        }
        0
    }
    let mut buf = samples.to_vec();
    let turns = unsafe {
        let result = sys::SherpaOnnxOfflineSpeakerDiarizationProcessWithCallback(
            sd.0,
            buf.as_mut_ptr(),
            buf.len() as i32,
            Some(trampoline),
            (&mut callback as *mut Box<dyn Fn(f32)>).cast(),
        );
        if result.is_null() {
            bail!("화자 분리에 실패했습니다");
        }
        let n = sys::SherpaOnnxOfflineSpeakerDiarizationResultGetNumSegments(result);
        let segments = sys::SherpaOnnxOfflineSpeakerDiarizationResultSortByStartTime(result);
        // 말소리를 하나도 못 찾으면(작게 말했거나 아주 짧은 녹음) 빈 결과다. 처리 과정이 녹음 전체를 한 사람으로 본다
        let turns = if segments.is_null() || n <= 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(segments, n as usize)
                .iter()
                .map(|s| SpeakerTurn { start: s.start, end: s.end, speaker: s.speaker })
                .collect()
        };
        if !segments.is_null() {
            sys::SherpaOnnxOfflineSpeakerDiarizationDestroySegment(segments);
        }
        sys::SherpaOnnxOfflineSpeakerDiarizationDestroyResult(result);
        turns
    };
    Ok(turns)
}
