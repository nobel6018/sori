//! CI에서 엔진 전체(디코딩, 화자 분리, 다시 묶기, 전사)가 실제로 도는지 확인한다.
//! 기대와 다르면 0이 아닌 코드로 끝난다
//!
//! cargo run --example smoke -- <모델 폴더>
//! 모델 폴더에는 ggml-tiny.bin과 화자 분리 모델 두 개가 있어야 한다
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use sori_lib::engine::{self, ModelPaths, ProcessOptions};
use sori_lib::models::{EMBEDDING_FILE, SEGMENTATION_FILE};

fn main() -> eyre::Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("모델 폴더"));
    let audio = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trilingual-40s.wav");
    let models = ModelPaths {
        whisper: dir.join("ggml-tiny.bin"),
        segmentation: dir.join(SEGMENTATION_FILE),
        embedding: dir.join(EMBEDDING_FILE),
    };
    let whisper = engine::load_whisper(&models.whisper)?;
    let t = engine::process(
        &audio,
        &whisper,
        &models,
        &ProcessOptions {
            num_speakers: Some(3),
            languages: vec!["ko".into(), "zh".into(), "en".into()],
            tracks: None,
            diarize_worker: None,
            diarize: true,
        },
        Arc::new(AtomicBool::new(false)),
        |_, _| {},
    )?;
    for u in &t.utterances {
        println!("[{:>5.1}] S{} ({}) {}", u.start, u.speaker, u.lang, u.text);
    }

    // 40초 구간: 민지(ko) 0.6초, David(en) 8.5초, 왕웨이(zh) 18.1초, 민지 27.9초, David 36.6초
    let speakers: std::collections::BTreeSet<i32> = t.utterances.iter().map(|u| u.speaker).collect();
    let langs: std::collections::BTreeSet<&str> = t.utterances.iter().map(|u| u.lang.as_str()).collect();
    let english = t.utterances.iter().filter(|u| u.lang == "en").map(|u| u.text.to_lowercase()).collect::<String>();
    let checks = [
        ("길이 40초", (t.duration - 40.0).abs() < 0.1),
        ("화자 3명", speakers.len() == 3),
        ("언어 3개(ko, zh, en)", langs.len() == 3),
        ("영어 문장 받아 적기(friday)", english.contains("friday")),
        ("민지의 두 발화가 같은 화자", same_speaker(&t, 1.0, 28.5)),
    ];
    let mut ok = true;
    for (name, pass) in checks {
        println!("{} {name}", if pass { "통과" } else { "실패" });
        ok &= pass;
    }
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn speaker_at(t: &engine::Transcript, time: f32) -> Option<i32> {
    t.utterances.iter().find(|u| u.start <= time && time < u.end + 0.5).map(|u| u.speaker)
}

fn same_speaker(t: &engine::Transcript, a: f32, b: f32) -> bool {
    matches!((speaker_at(t, a), speaker_at(t, b)), (Some(x), Some(y)) if x == y)
}
