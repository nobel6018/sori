//! 화자 수 자동 추정 기준값 맞추기: 화자 수를 아는 녹음마다 기준값별로 몇 명이 나오는지 본다
//! cargo run --release --example speaker_count_check -- <임베딩 모델> <녹음 폴더>=<화자 수> ...
//! 녹음 폴더의 transcript.json 구간 경계와 audio 파일만 쓴다. 받아 적은 내용은 읽지 않는다
use std::path::PathBuf;

use sori_lib::engine::recluster::{cluster, drop_minor, turn_embeddings, Stop};
use sori_lib::engine::{audio, Transcript};

fn main() -> eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let model = PathBuf::from(&args[1]);
    let thresholds: Vec<f32> = (6..=14).map(|i| i as f32 * 0.05).collect();
    println!("{:<12} 정답  {}", "녹음", thresholds.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join("  "));
    for spec in &args[2..] {
        let (dir, truth) = spec.rsplit_once('=').unwrap();
        let dir = PathBuf::from(dir);
        let t: Transcript = serde_json::from_slice(&std::fs::read(dir.join("transcript.json"))?)?;
        let audio_path = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.file_stem().is_some_and(|s| s == "audio"))
            .unwrap();
        let samples = audio::load_mono_16k(&audio_path)?;
        let emb = turn_embeddings(&t.turns, &samples, &model)?;
        let anchors: Vec<usize> = (0..t.turns.len()).filter(|&i| emb[i].is_some() && t.turns[i].end - t.turns[i].start >= 1.0).collect();
        let vecs: Vec<&[f32]> = anchors.iter().map(|&i| emb[i].as_deref().unwrap()).collect();
        let durs: Vec<f32> = anchors.iter().map(|&i| t.turns[i].end - t.turns[i].start).collect();
        let counts: Vec<String> = thresholds
            .iter()
            .map(|&th| {
                let labels = drop_minor(&vecs, &durs, cluster(&vecs, Stop::Below(th)));
                format!("{:>4}", labels.iter().max().map_or(0, |m| m + 1))
            })
            .collect();
        let name = dir.file_name().unwrap().to_string_lossy()[..8].to_string();
        println!("{name:<12} {truth:>3}  {}   (구간 {}개)", counts.join("  "), anchors.len());
    }
    Ok(())
}
