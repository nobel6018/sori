//! 화자 분리 작업 프로세스 확인: 끝까지 돌린 결과와, 3초 뒤 취소했을 때 멈추는 데 걸린 시간을 본다
//! cargo run --example worker_check -- <sori 실행 파일> <오디오> <구간 모델> <임베딩 모델>
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sori_lib::engine::worker::diarize_in_child;

fn main() {
    let a: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let never = AtomicBool::new(false);
    let t = Instant::now();
    match diarize_in_child(&a[0], &a[1], &a[2], &a[3], None, &never, |_| {}) {
        Ok(turns) => println!("끝까지: {:.1}s, 구간 {}개", t.elapsed().as_secs_f32(), turns.len()),
        Err(e) => println!("끝까지: 오류 {e:#}"),
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let started = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        flag.store(true, Ordering::Relaxed);
    });
    let r = diarize_in_child(&a[0], &a[1], &a[2], &a[3], None, &cancel, |_| {});
    println!("3초 뒤 취소: {:.2}s 만에 끝남, {:?}", started.elapsed().as_secs_f32(), r.err().map(|e| e.to_string()));
}
