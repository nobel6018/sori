//! 회의 앱 감지 확인: 오디오를 쓰는 프로세스와 마이크 사용 여부, 알아본 회의 앱을 보여 준다
//! cargo run --example meeting_check
fn main() {
    #[cfg(target_os = "macos")]
    for (bundle, input) in sori_lib::meeting::audio_processes() {
        println!("{bundle:50} 마이크 {}", if input { "사용 중" } else { "-" });
    }
    println!("회의 앱: {:?}", sori_lib::meeting::meeting_apps_using_mic());
}
