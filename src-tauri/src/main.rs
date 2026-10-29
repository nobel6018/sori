// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 화자 분리 작업 프로세스로 띄운 경우. 창을 만들지 않고 결과만 내고 끝난다(engine/worker.rs)
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(sori_lib::engine::worker::ARG) {
        std::process::exit(sori_lib::engine::worker::run_worker(&args[2..]));
    }
    sori_lib::run()
}
