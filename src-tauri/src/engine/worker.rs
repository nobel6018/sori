//! 화자 분리를 별도 프로세스로 돌린다. sherpa-onnx는 화자 분리 도중 멈출 수 없어서,
//! 같은 프로세스에서 돌리면 취소가 그 단계가 끝날 때까지(15분 녹음에 1분 넘게) 늦고,
//! 그사이 앱을 끄면 종료 정리와 엉켜 앱이 비정상 종료된다. 프로세스를 끝내면 바로 멈춘다.
//! 앱 실행 파일을 `--diarize-worker` 인자로 다시 띄운다(src/main.rs).
//! @see docs/processing.md

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eyre::{bail, eyre, Result, WrapErr};

use super::audio;
use super::diarize::{self, DiarizeOptions, SpeakerTurn};

pub const ARG: &str = "--diarize-worker";

/// 작업 프로세스 쪽. 인자: 오디오, 구간 모델, 임베딩 모델, 화자 수("auto" 또는 숫자).
/// 진행률은 표준 에러에 "progress 0.42" 줄로, 결과는 표준 출력에 JSON으로 낸다. 종료 코드를 돌려준다
pub fn run_worker(args: &[String]) -> i32 {
    let result = (|| -> Result<Vec<SpeakerTurn>> {
        let [audio_path, segmentation, embedding, speakers] = args else {
            bail!("인자가 맞지 않습니다");
        };
        let samples = audio::load_mono_16k(Path::new(audio_path))?;
        let num_speakers = speakers.parse::<i32>().ok();
        diarize::diarize(
            &samples,
            Path::new(segmentation),
            Path::new(embedding),
            &DiarizeOptions { num_speakers, ..Default::default() },
            |p| eprintln!("progress {p}"),
        )
    })();
    match result {
        Ok(turns) => match serde_json::to_string(&turns) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(e) => {
                eprintln!("error {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("error {e:#}");
            1
        }
    }
}

/// 앱 쪽. 작업 프로세스를 띄워 결과를 기다리고, cancel이 켜지면 프로세스를 끝낸다
pub fn diarize_in_child(
    exe: &Path,
    audio_path: &Path,
    segmentation: &Path,
    embedding: &Path,
    num_speakers: Option<i32>,
    cancel: &AtomicBool,
    on_progress: impl Fn(f32) + Send + 'static,
) -> Result<Vec<SpeakerTurn>> {
    let mut cmd = Command::new(exe);
    cmd.arg(ARG)
        .arg(audio_path)
        .arg(segmentation)
        .arg(embedding)
        .arg(num_speakers.map_or_else(|| "auto".to_string(), |n| n.to_string()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
        return Err(eyre!(if last_error.is_empty() { format!("화자 분리 프로세스가 멈췄습니다({status})") } else { last_error }));
    }
    serde_json::from_str(json.trim()).wrap_err("화자 분리 결과를 읽지 못했습니다")
}

/// 작업 프로세스로 쓸 실행 파일. 테스트와 예제에서는 None이라 같은 프로세스에서 돌린다
pub fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}
