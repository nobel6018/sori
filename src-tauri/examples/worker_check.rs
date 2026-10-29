//! 작업 프로세스 확인: 화자 찾기를 끝까지 돌린 결과와, 3초 뒤 취소했을 때 멈추는 데 걸린 시간을 본다
//! cargo run --example worker_check -- <sori 실행 파일> <오디오> <구간 모델> <임베딩 모델>
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sori_lib::engine::worker::{embeddings_in_child, speakers_in_child};
use sori_lib::engine::ModelPaths;

fn main() {
    let a: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let models = ModelPaths { whisper: PathBuf::new(), segmentation: a[2].clone(), embedding: a[3].clone() };
    let t = Instant::now();
    match speakers_in_child(&a[0], &a[1], &models, None, &AtomicBool::new(false), |_| {}) {
        Ok(s) => {
            let n = s.turns.iter().map(|x| x.speaker).collect::<std::collections::BTreeSet<_>>().len();
            println!("끝까지: {:.1}s, 구간 {}개, 화자 {n}명, 목소리 특징 {}개", t.elapsed().as_secs_f32(), s.turns.len(), s.speaker_embeddings.len());
            match embeddings_in_child(&a[0], &a[1], &a[3], &s.turns) {
                Ok(e) => println!("목소리 특징만: {}개", e.len()),
                Err(e) => println!("목소리 특징만: 오류 {e:#}"),
            }
        }
        Err(e) => println!("끝까지: 오류 {e:#}"),
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let started = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        flag.store(true, Ordering::Relaxed);
    });
    let r = speakers_in_child(&a[0], &a[1], &models, None, &cancel, |_| {});
    println!("3초 뒤 취소: {:.2}s 만에 끝남, {:?}", started.elapsed().as_secs_f32(), r.err().map(|e| e.to_string()));
    let bad = speakers_in_child(&a[0], &PathBuf::from("/없는/파일.wav"), &models, None, &AtomicBool::new(false), |_| {});
    println!("없는 파일: {:?}", bad.err().map(|e| e.to_string()));
}
