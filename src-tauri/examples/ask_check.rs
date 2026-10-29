//! 녹음에 질문하기 확인: 녹음 폴더 하나에 질문을 보내 답과 시각 표시를 본다
//! cargo run --example ask_check -- <녹음 폴더> "<질문>"
use std::path::PathBuf;

use sori_lib::store::Recording;
use sori_lib::{ask, engine::Transcript, summarize};

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let recording: Recording = serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
    let transcript: Transcript = serde_json::from_slice(&std::fs::read(dir.join("transcript.json"))?)?;
    let prompt = ask::single_prompt(&args[2], &recording, &transcript, None);
    let (_tx, rx) = tokio::sync::oneshot::channel();
    println!("{}", summarize::run(summarize::Provider::Claude, prompt, rx).await?);
    Ok(())
}
