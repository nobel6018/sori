//! 화자 분리만 돌려 구간 수를 본다. 전사는 하지 않는다
//! cargo run --release --example diarize_check -- <오디오> <구간 모델> <임베딩 모델>
use std::path::PathBuf;

use sori_lib::engine::{audio, diarize};

fn main() -> eyre::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let samples = audio::load_mono_16k(&PathBuf::from(&a[1]))?;
    println!("길이 {:.1}s", samples.len() as f32 / 16000.0);
    for n in [None, Some(1), Some(2)] {
        let r = diarize::diarize(&samples, &PathBuf::from(&a[2]), &PathBuf::from(&a[3]), &diarize::DiarizeOptions { num_speakers: n, ..Default::default() }, |_| {});
        match r {
            Ok(t) => println!("화자 수 {n:?}: 구간 {}개, 화자 {:?}", t.len(), t.iter().map(|x| x.speaker).collect::<std::collections::BTreeSet<_>>()),
            Err(e) => println!("화자 수 {n:?}: 오류 {e}"),
        }
    }
    Ok(())
}
