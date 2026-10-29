// 모바일은 lib.rs의 run이 진입점이다. 데스크톱에서 cargo run으로 화면을 확인할 때만 쓴다
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sori_mobile_lib::run()
}
