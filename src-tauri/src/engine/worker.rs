//! onnxruntime을 쓰는 일(화자 분리, 다시 묶기, 목소리 특징)을 별도 프로세스로 돌린다.
//! - sherpa-onnx는 화자 분리 도중 멈출 수 없어, 같은 프로세스에서 돌리면 취소가 그 단계가 끝날 때까지 늦었다
//! - onnxruntime이 C++ 예외(Ort::Exception)를 던지면 받아 줄 곳이 없어 앱 전체가 비정상 종료됐다(0.4.1 크래시 기록)
//! 프로세스를 나누면 취소는 프로세스를 끝내는 것으로 바로 되고, 예외가 나도 작업 프로세스만 죽고 앱은 오류를 보여 준다.
//! 앱 실행 파일을 `--diarize-worker` 인자로 다시 띄운다(src/main.rs).
//! @see docs/processing.md

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eyre::{bail, eyre, Result, WrapErr};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::diarize::SpeakerTurn;
use super::{audio, find_speakers, voiceprint, ModelPaths, Speakers};

pub const ARG: &str = "--diarize-worker";

/// 작업 프로세스 쪽. 첫 인자가 할 일이다.
/// - `speakers <오디오> <구간 모델> <임베딩 모델> <화자 수|auto>`: 화자 찾기 결과(Speakers)
/// - `embed <오디오> <임베딩 모델> <구간 JSON 파일>`: 화자별 목소리 특징
///
/// 진행률은 표준 에러에 "progress 0.42" 줄로, 결과는 표준 출력에 JSON으로 낸다. 종료 코드를 돌려준다
pub fn run_worker(args: &[String]) -> i32 {
    quiet_abort();
    fn emit<T: Serialize>(result: Result<T>) -> i32 {
        match result.and_then(|v| Ok(serde_json::to_string(&v)?)) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(e) => {
                eprintln!("error {e:#}");
                1
            }
        }
    }
    match args {
        [mode, audio_path, segmentation, embedding, speakers] if mode == "speakers" => emit((|| {
            let samples = audio::load_mono_16k(Path::new(audio_path))?;
            let models = ModelPaths { whisper: Default::default(), segmentation: segmentation.into(), embedding: embedding.into() };
            find_speakers(&samples, &models, speakers.parse::<i32>().ok(), |p| eprintln!("progress {p}"))
        })()),
        [mode, audio_path, embedding, turns_path] if mode == "embed" => emit((|| {
            let samples = audio::load_mono_16k(Path::new(audio_path))?;
            let turns: Vec<SpeakerTurn> = serde_json::from_slice(&std::fs::read(turns_path)?)?;
            voiceprint::speaker_embeddings(&turns, &samples, Path::new(embedding))
        })()),
        _ => {
            eprintln!("error 작업 프로세스 인자가 맞지 않습니다");
            1
        }
    }
}

/// onnxruntime 예외로 abort가 나면 macOS가 크래시 기록을 남기고 "sori가 예기치 않게 종료됨" 창을 띄운다.
/// 작업 프로세스는 앱과 같은 실행 파일이라 사용자에게는 앱이 죽은 것처럼 보인다. abort 신호를 받으면
/// 바로 끝낸다. 원인(libc++abi의 "terminating due to uncaught exception ...")은 그 전에 표준 에러로 나가 앱이 받는다
fn quiet_abort() {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
            fn _exit(code: i32) -> !;
        }
        extern "C" fn on_abort(_: i32) {
            unsafe { _exit(134) }
        }
        const SIGABRT: i32 = 6;
        unsafe { signal(SIGABRT, on_abort) };
    }
}

/// 앱 쪽: 화자 찾기를 작업 프로세스에서 한다. cancel이 켜지면 프로세스를 끝낸다
pub fn speakers_in_child(
    exe: &Path,
    audio_path: &Path,
    models: &ModelPaths,
    num_speakers: Option<i32>,
    cancel: &AtomicBool,
    on_progress: impl Fn(f32) + Send + 'static,
) -> Result<Speakers> {
    let speakers = num_speakers.map_or_else(|| "auto".to_string(), |n| n.to_string());
    run_child(
        exe,
        &["speakers".as_ref(), audio_path.as_os_str(), models.segmentation.as_os_str(), models.embedding.as_os_str(), speakers.as_ref()],
        cancel,
        on_progress,
    )
}

/// 앱 쪽: 정해진 구간의 화자별 목소리 특징을 작업 프로세스에서 뽑는다
pub fn embeddings_in_child(exe: &Path, audio_path: &Path, embedding: &Path, turns: &[SpeakerTurn]) -> Result<BTreeMap<i32, Vec<f32>>> {
    let file = std::env::temp_dir().join(format!("sori-turns-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec(turns)?)?;
    let result = run_child(
        exe,
        &["embed".as_ref(), audio_path.as_os_str(), embedding.as_os_str(), file.as_os_str()],
        &AtomicBool::new(false),
        |_| {},
    );
    let _ = std::fs::remove_file(&file);
    result
}

fn run_child<T: DeserializeOwned>(
    exe: &Path,
    args: &[&std::ffi::OsStr],
    cancel: &AtomicBool,
    on_progress: impl Fn(f32) + Send + 'static,
) -> Result<T> {
    let mut cmd = Command::new(exe);
    cmd.arg(ARG).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 콘솔 창이 잠깐 뜨지 않게 한다
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().wrap_err("화자 분리 프로세스를 띄우지 못했습니다")?;

    let stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut last_error = String::new();
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            if let Some(p) = line.strip_prefix("progress ").and_then(|p| p.parse::<f32>().ok()) {
                on_progress(p);
            } else if let Some(e) = line.strip_prefix("error ") {
                last_error = e.to_string();
            } else if line.contains("terminat") || line.contains("Exception") {
                // onnxruntime이 예외로 죽으면 표준 에러에 남긴 마지막 줄이 원인이다
                last_error = line;
            }
        }
        last_error
    });
    let mut stdout = child.stdout.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("취소했습니다");
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let last_error = errors.join().unwrap_or_default();
    let json = output.join().unwrap_or_default();
    if !status.success() {
        return Err(eyre!(if last_error.is_empty() {
            format!("화자 분리 프로세스가 비정상 종료했습니다({status}). 화자 수를 바꾸거나 \"전사만\"으로 처리해 보세요")
        } else {
            format!("화자 분리에 실패했습니다: {last_error}")
        }));
    }
    serde_json::from_str(json.trim()).wrap_err("화자 분리 결과를 읽지 못했습니다")
}

/// 작업 프로세스로 쓸 실행 파일. 테스트와 예제에서는 None이라 같은 프로세스에서 돌린다
pub fn current_exe() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok()
}
