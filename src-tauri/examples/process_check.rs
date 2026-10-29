//! 처리 과정 전체를 돌려 발화 수와 시간만 본다. 받아 적은 내용은 출력하지 않는다
//! cargo run --release --example process_check -- <오디오> <settings.json>
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

use sori_lib::engine::{self, ProcessOptions};
use sori_lib::store::Settings;

fn main() -> eyre::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let settings: Settings = serde_json::from_slice(&std::fs::read(&a[2])?)?;
    let whisper = engine::load_whisper(&settings.models.whisper)?;
    let options = ProcessOptions { num_speakers: None, languages: settings.default_languages.clone(), tracks: None };
    let t = engine::process(&PathBuf::from(&a[1]), &whisper, &settings.models, &options, Arc::new(AtomicBool::new(false)), |_, _| {})?;
    println!("길이 {:.1}s, 화자 구간 {}개, 발화 {}개", t.duration, t.turns.len(), t.utterances.len());
    for u in &t.utterances {
        println!("  {:.1}~{:.1}s 화자 {} 언어 {} 글자 {}개", u.start, u.end, u.speaker, u.lang, u.text.chars().count());
    }
    Ok(())
}
