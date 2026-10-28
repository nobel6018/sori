//! 앱 없이 엔진만 돌려 보는 확인용 실행 파일
//! cargo run --release --example pipeline -- <오디오 파일> [화자 수|auto] [ko,zh]
//! SORI_WHISPER=<ggml 모델 경로>로 Whisper 모델을 바꿀 수 있다
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use sori_lib::engine::{self, ModelPaths, ProcessOptions};

fn main() -> eyre::Result<()> {
    let mut args = std::env::args().skip(1);
    let audio = PathBuf::from(args.next().expect("오디오 파일 경로"));
    let num_speakers = args.next().filter(|n| n != "auto").map(|n| n.parse()).transpose()?;
    let languages: Vec<String> = args.next().map(|l| l.split(',').map(String::from).collect()).unwrap_or_default();

    let home = PathBuf::from(std::env::var("HOME")?);
    let models = ModelPaths {
        // SORI_WHISPER로 다른 Whisper 모델(예: ggml-large-v3.bin)을 시험한다
        whisper: std::env::var("SORI_WHISPER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("whisper-models/ggml-large-v3-turbo.bin")),
        segmentation: home.join("sherpa-onnx/models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
        embedding: home.join("sherpa-onnx/models/3dspeaker_eres2net_base_zh-cn_16k.onnx"),
    };

    let started = Instant::now();
    let whisper = engine::load_whisper(&models.whisper)?;
    eprintln!("모델 로딩 {:.1}s", started.elapsed().as_secs_f32());

    let t = engine::process(
        &audio,
        &whisper,
        &models,
        &ProcessOptions { num_speakers, languages, tracks: None },
        Arc::new(AtomicBool::new(false)),
        |stage, p| eprint!("\r{stage:?} {:>3.0}%   ", p * 100.0),
    )?;
    eprintln!("\n전체 {:.1}s, 오디오 {:.1}s, 화자 구간 {}개", started.elapsed().as_secs_f32(), t.duration, t.turns.len());
    for u in &t.utterances {
        println!("[{:>6.1} - {:>6.1}] S{} ({}) {}", u.start, u.end, u.speaker, u.lang, u.text);
    }
    Ok(())
}
