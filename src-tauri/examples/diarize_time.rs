//! 화자 분리 시간만 잰다. 구간 수만 출력하고 내용은 보지 않는다
use std::path::PathBuf;
use std::time::Instant;
use sori_lib::engine::{audio, diarize};
fn main() -> eyre::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let t0 = Instant::now();
    let samples = audio::load_mono_16k(&PathBuf::from(&a[1]))?;
    println!("디코딩 {:.1}s, 길이 {:.0}s", t0.elapsed().as_secs_f32(), samples.len() as f32 / 16000.0);
    let t1 = Instant::now();
    let last = std::sync::Mutex::new(Instant::now());
    let turns = diarize::diarize(&samples, &PathBuf::from(&a[2]), &PathBuf::from(&a[3]), &diarize::DiarizeOptions::default(), move |p| {
        let mut l = last.lock().unwrap();
        if l.elapsed().as_secs_f32() > 5.0 { println!("  진행 {:.0}% ({:.0}s)", p * 100.0, t1.elapsed().as_secs_f32()); *l = Instant::now(); }
    })?;
    println!("화자 분리 {:.1}s, 구간 {}개", t1.elapsed().as_secs_f32(), turns.len());
    Ok(())
}
