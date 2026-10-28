//! 앱 없이 저장된 녹음 하나를 요약해 본다
//! cargo run --example summarize -- <녹음 폴더> [claude|codex]
use std::path::PathBuf;

use sori_lib::engine::Transcript;
use sori_lib::store::Recording;
use sori_lib::summarize::{self, Provider};

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("녹음 폴더 경로"));
    let provider = match args.next().as_deref() {
        Some("codex") => Provider::Codex,
        _ => Provider::Claude,
    };
    for s in summarize::statuses() {
        eprintln!("{:?}: {:?}", s.provider, s.path);
    }
    let recording: Recording = serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
    let transcript: Transcript = serde_json::from_slice(&std::fs::read(dir.join("transcript.json"))?)?;
    let prompt = summarize::build_prompt(summarize::DEFAULT_PROMPT, &recording, &transcript);
    let (_tx, rx) = tokio::sync::oneshot::channel();
    let started = std::time::Instant::now();
    let summary = summarize::run(provider, prompt, rx).await?;
    eprintln!("{:.1}s", started.elapsed().as_secs_f32());
    println!("{summary}");
    Ok(())
}
