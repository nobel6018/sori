//! 종료 크래시 재현: whisper(Metal)를 한 번 돌린 뒤 모델을 들고 있는 채로 끝내면 ggml Metal 장치의
//! 전역 정리에서 abort가 나는지 본다. 두 번째 인자가 "drop"이면 끝내기 전에 모델을 해제한다
//! cargo run --release --example exit_check -- <whisper 모델> [drop]
use sori_lib::engine;

fn main() -> eyre::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let ctx = engine::load_whisper(std::path::Path::new(&a[1]))?;
    let mut state = ctx.create_state()?;
    let params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    state.full(params, &vec![0.0f32; 16000 * 2])?;
    drop(state);
    if a.get(2).map(String::as_str) == Some("drop") {
        drop(ctx);
        println!("모델을 해제하고 끝냅니다");
    } else {
        // 앱처럼 모델을 끝까지 들고 있는다
        Box::leak(Box::new(ctx));
        println!("모델을 들고 끝냅니다");
    }
    Ok(())
}
