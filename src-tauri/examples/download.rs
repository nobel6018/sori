//! 모델 내려받기만 따로 시험한다. 중간에 취소한 뒤 다시 받아 이어 받기도 확인한다
//! cargo run --example download -- <받을 폴더> [취소할 바이트]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use sori_lib::download::{download_all, Item};

fn main() -> eyre::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("받을 폴더"));
    let cancel_at: Option<u64> = args.next().map(|n| n.parse()).transpose()?;
    let items = [Item {
        name: "화자 임베딩 모델".into(),
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx".into(),
        dest: dir.join("emb.onnx"),
        size: 39_593_761,
    }];
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let result = download_all(&items, &cancel, |p| {
        if cancel_at.is_some_and(|at| p.received >= at) {
            cancel.store(true, Ordering::Relaxed);
        }
        eprint!("\r{:>5.1}MB / {:.1}MB   ", p.received as f64 / 1e6, p.total as f64 / 1e6);
    });
    eprintln!("\n{:.1}s {:?}", started.elapsed().as_secs_f32(), result.as_ref().map_err(|e| format!("{e:#}")));
    Ok(())
}
