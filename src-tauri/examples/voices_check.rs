//! 목소리 기억 기준값 확인: 한 녹음을 앞뒤 절반으로 나눠 화자별 특징을 뽑고 서로 비교한다
//! cargo run --release --example voices_check -- <녹음 폴더> <임베딩 모델>
use std::path::PathBuf;

use sori_lib::engine::{self, voiceprint::similarity, Transcript};

fn main() -> eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let model = PathBuf::from(&args[2]);
    let t: Transcript = serde_json::from_slice(&std::fs::read(dir.join("transcript.json"))?)?;
    let audio = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.file_stem().is_some_and(|s| s == "audio"))
        .expect("audio 파일");
    let half = t.duration / 2.0;
    let first: Vec<_> = t.turns.iter().filter(|x| x.end <= half).cloned().collect();
    let second: Vec<_> = t.turns.iter().filter(|x| x.start >= half).cloned().collect();
    let a = engine::embeddings_for(&audio, &first, &model, None)?;
    let b = engine::embeddings_for(&audio, &second, &model, None)?;
    println!("앞 절반 화자 {:?}, 뒤 절반 화자 {:?}", a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (sa, ea) in &a {
        let row: Vec<String> = b.iter().map(|(sb, eb)| format!("{sb}:{:.2}", similarity(ea, eb))).collect();
        println!("앞 화자 {sa} → {}", row.join("  "));
    }
    Ok(())
}
