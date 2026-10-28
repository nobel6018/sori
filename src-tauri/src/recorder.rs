use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use eyre::{eyre, Result, WrapErr};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_input_devices() -> Result<Vec<InputDevice>> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok()).map(|id| id.to_string());
    let mut devices = Vec::new();
    for device in host.input_devices()? {
        let Ok(id) = device.id() else { continue };
        let id = id.to_string();
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| id.clone());
        devices.push(InputDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    Ok(devices)
}

/// 녹음할 소리. 마이크와 시스템 소리는 각자 트랙 파일로 따로 저장한다
#[derive(Debug, Clone)]
pub enum Source {
    /// None이면 시스템 기본 마이크
    Mic(Option<String>),
    /// 기본 출력 장치로 나가는 소리(통화 상대 목소리 등). macOS 14.6 이상은 Core Audio 탭, Windows는 WASAPI 루프백
    System,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub source: Source,
    pub path: PathBuf,
}

enum Event {
    Samples(usize, Vec<f32>),
    StreamError(String),
}

/// 녹음 중인 스레드의 핸들. stop을 부르거나 drop하면 녹음을 끝내고 파일을 마무리한다
pub struct ActiveRecording {
    stop_tx: mpsc::Sender<()>,
    handle: Option<JoinHandle<Result<Finished>>>,
}

pub struct Finished {
    /// 장치가 빠지는 등 녹음 도중 끊긴 이유. 그 전까지의 녹음은 파일에 남아 있다
    pub interrupted: Option<String>,
}

impl ActiveRecording {
    pub fn stop(mut self) -> Result<Finished> {
        let _ = self.stop_tx.send(());
        match self.handle.take() {
            Some(h) => h.join().map_err(|_| eyre!("녹음 스레드가 비정상 종료됐습니다"))?,
            None => Err(eyre!("이미 끝난 녹음입니다")),
        }
    }
}

impl Drop for ActiveRecording {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 이 간격마다 WAV 헤더를 갱신해, 앱이 비정상 종료돼도 그때까지의 녹음은 재생된다
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// 레벨 미터 이벤트 간격
const LEVEL_INTERVAL: Duration = Duration::from_millis(66);

type Writer = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

/// 트랙마다 장치 기본 샘플레이트 그대로 16비트 모노 WAV로 녹음한다.
/// 리샘플링은 트랙을 섞거나 전사할 때 하면 되므로 녹음 중에는 하지 않는다
pub fn start(
    tracks: Vec<Track>,
    on_level: impl Fn(Vec<f32>) + Send + 'static,
    on_interrupted: impl FnOnce(String) + Send + 'static,
) -> Result<ActiveRecording> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();

    // cpal 스트림은 macOS에서 다른 스레드로 옮길 수 없어서, 만들고 끝낼 때까지 한 스레드가 소유한다
    let handle = std::thread::spawn(move || run(tracks, stop_rx, ready_tx, on_level, on_interrupted));

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(ActiveRecording {
            stop_tx,
            handle: Some(handle),
        }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err(eyre!("녹음 스레드를 시작하지 못했습니다")),
    }
}

fn run(
    tracks: Vec<Track>,
    stop_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::Sender<Result<()>>,
    on_level: impl Fn(Vec<f32>),
    on_interrupted: impl FnOnce(String),
) -> Result<Finished> {
    let (tx, rx) = mpsc::channel::<Event>();
    let setup = || -> Result<(Vec<cpal::Stream>, Vec<Writer>, Vec<u32>)> {
        let mut streams = Vec::new();
        let mut writers = Vec::new();
        let mut rates = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            let (stream, rate) = open_stream(&track.source, index, tx.clone())?;
            rates.push(rate);
            writers.push(hound::WavWriter::create(
                &track.path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )?);
            streams.push(stream);
        }
        // 트랙 사이 시작 시점 차이를 줄이려고 장치를 다 연 뒤 한꺼번에 시작한다
        for (stream, track) in streams.iter().zip(&tracks) {
            stream.play().wrap_err_with(|| match track.source {
                Source::Mic(_) => "마이크 녹음을 시작하지 못했습니다. 마이크 권한을 확인해 주세요",
                Source::System => "시스템 소리 녹음을 시작하지 못했습니다. 시스템 오디오 녹음 권한을 확인해 주세요",
            })?;
        }
        Ok((streams, writers, rates))
    };

    let (streams, mut writers, rates) = match setup() {
        Ok(v) => {
            let _ = ready_tx.send(Ok(()));
            v
        }
        Err(e) => {
            let message = format!("{e:#}");
            let _ = ready_tx.send(Err(e));
            return Err(eyre!(message));
        }
    };

    let _awake = KeepAwake::begin();
    let started = Instant::now();
    let mut written = vec![0u64; writers.len()];
    let mut peaks = vec![0.0f32; writers.len()];
    let mut last_flush = Instant::now();
    let mut last_level = Instant::now();
    let mut interrupted = None;

    // 쓰기 실패(디스크 가득 참 등)도 장치 오류처럼 "중간에 끊김"으로 다룬다. 그때까지 쓴 파일은 아래에서 마무리한다
    let result = (|| -> Result<()> {
        loop {
            if stop_rx.try_recv().is_ok() {
                return Ok(());
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::Samples(i, samples)) => {
                    write_samples(&mut writers[i], &samples)?;
                    peaks[i] = samples.iter().fold(peaks[i], |p, s| p.max(s.abs()));
                    written[i] += samples.len() as u64;
                }
                Ok(Event::StreamError(e)) => {
                    interrupted = Some(e);
                    return Ok(());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            pad_gaps(&mut writers, &mut written, &rates, started.elapsed())?;
            if last_level.elapsed() >= LEVEL_INTERVAL {
                on_level(peaks.iter().map(|p| level(*p)).collect());
                peaks.iter_mut().for_each(|p| *p = 0.0);
                last_level = Instant::now();
            }
            if last_flush.elapsed() >= FLUSH_INTERVAL {
                for w in &mut writers {
                    w.flush()?;
                }
                last_flush = Instant::now();
            }
        }
    })();
    if let Err(e) = result {
        interrupted = Some(format!("녹음 파일을 쓰지 못했습니다. 디스크 공간을 확인해 주세요: {e:#}"));
    }

    // 스트림을 먼저 멈춘 뒤 채널에 남은 샘플까지 받아 적는다
    drop(streams);
    while let Ok(event) = rx.try_recv() {
        if let Event::Samples(i, samples) = event {
            if write_samples(&mut writers[i], &samples).is_ok() {
                written[i] += samples.len() as u64;
            }
        }
    }
    let _ = pad_gaps(&mut writers, &mut written, &rates, started.elapsed());
    let mut finalize_error = None;
    for w in writers {
        if let Err(e) = w.finalize() {
            finalize_error.get_or_insert(e);
        }
    }
    on_level(vec![0.0; written.len()]);
    // 사용자가 멈추기를 누르지 않았는데 끝났으면 화면에 알려 멈추기 처리를 하게 한다
    if let Some(reason) = &interrupted {
        on_interrupted(reason.clone());
    }
    if let Some(e) = finalize_error {
        return Err(e.into());
    }

    Ok(Finished { interrupted })
}

/// 트랙이 실제 흐른 시간보다 이만큼 넘게 짧아지면 무음으로 채운다
const GAP_THRESHOLD: Duration = Duration::from_millis(500);

/// Windows WASAPI 루프백은 아무 소리도 나지 않는 동안 데이터를 보내지 않는다. 그대로 두면 상대가 조용했던
/// 시간만큼 시스템 소리 트랙이 짧아지고, 섞을 때 그 뒤 원격 목소리가 모두 앞당겨진다.
/// 트랙마다 흐른 시간에 비해 모자란 만큼을 무음으로 채워 트랙 길이를 실제 시간에 맞춘다.
/// 블루투스 마이크가 잠깐 끊길 때도 같은 일이 생긴다. 장치 버퍼 지연(수십 ms)으로는 채우지 않도록 여유를 둔다
fn pad_gaps(writers: &mut [Writer], written: &mut [u64], rates: &[u32], elapsed: Duration) -> Result<()> {
    for ((w, n), &rate) in writers.iter_mut().zip(written.iter_mut()).zip(rates) {
        let expected = (elapsed.as_secs_f64() * rate as f64) as u64;
        let threshold = (GAP_THRESHOLD.as_secs_f64() * rate as f64) as u64;
        if expected > *n + threshold {
            // 곧 도착할 샘플 몫으로 50ms는 남겨 둔다
            let fill = expected - *n - rate as u64 / 20;
            for _ in 0..fill {
                w.write_sample(0i16)?;
            }
            *n += fill;
        }
    }
    Ok(())
}

fn write_samples(writer: &mut Writer, samples: &[f32]) -> Result<()> {
    for s in samples {
        writer.write_sample(i16::from_sample(s.clamp(-1.0, 1.0)))?;
    }
    Ok(())
}

fn open_stream(source: &Source, index: usize, tx: mpsc::Sender<Event>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let (device, config) = match source {
        Source::Mic(device_id) => {
            let device = match device_id {
                Some(id) => {
                    let id = id.parse().map_err(|e| eyre!("잘못된 장치 id입니다: {e}"))?;
                    host.device_by_id(&id)
                }
                None => host.default_input_device(),
            }
            .ok_or_else(|| eyre!("마이크를 찾을 수 없습니다"))?;
            let config = device.default_input_config().wrap_err("마이크 설정을 읽지 못했습니다")?;
            (device, config)
        }
        Source::System => {
            let device = host
                .default_output_device()
                .ok_or_else(|| eyre!("소리를 내보내는 출력 장치를 찾을 수 없습니다"))?;
            // cpal은 macOS에서 입력도 되는 출력 장치(일부 헤드셋)를 루프백 대신 그 장치의 마이크로 연다.
            // 그러면 시스템 소리 대신 마이크 소리가 녹음되므로 미리 막는다
            if cfg!(target_os = "macos") && device.supports_input() {
                let name = device.description().map(|d| d.name().to_string()).unwrap_or_default();
                return Err(eyre!(
                    "지금 출력 장치({name})는 시스템 소리 녹음을 지원하지 않습니다. 출력을 다른 장치로 바꾸거나 시스템 소리 녹음을 꺼 주세요"
                ));
            }
            let config = device
                .default_output_config()
                .wrap_err("출력 장치 설정을 읽지 못했습니다")?;
            (device, config)
        }
    };

    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config.into(), channels, index, tx)?,
        SampleFormat::I16 => build::<i16>(&device, config.into(), channels, index, tx)?,
        SampleFormat::I32 => build::<i32>(&device, config.into(), channels, index, tx)?,
        SampleFormat::U16 => build::<u16>(&device, config.into(), channels, index, tx)?,
        other => return Err(eyre!("지원하지 않는 샘플 형식입니다: {other}")),
    };
    Ok((stream, rate))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    index: usize,
    tx: mpsc::Sender<Event>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let err_tx = tx.clone();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &_| {
            let mono: Vec<f32> = data
                .chunks_exact(channels)
                .map(|frame| frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32)
                .collect();
            let _ = tx.send(Event::Samples(index, mono));
        },
        move |err: cpal::Error| match err.kind() {
            // 몇 ms가 빠지는 경고라 녹음은 계속한다. 앱이 백그라운드에서 느려질 때 자주 난다
            cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceChanged => {}
            _ => {
                let _ = err_tx.send(Event::StreamError(err.to_string()));
            }
        },
        None,
    )?;
    Ok(stream)
}

/// 녹음하는 동안 잠자기를 막는다. 회의 중에는 앱이 대부분 뒤에 가려져 있고 입력도 없어서,
/// macOS는 App Nap으로 녹음 스레드를 느리게 돌리고 Windows는 유휴 시간이 지나면 잠자기에 들어간다.
/// 녹음 스레드 안에서 만들고 없애야 한다(Windows 실행 상태는 스레드 단위다)
struct KeepAwake {
    #[cfg(target_os = "macos")]
    activity: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>,
}

impl KeepAwake {
    #[cfg(target_os = "macos")]
    fn begin() -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical,
            &NSString::from_str("녹음 중"),
        );
        Self { activity }
    }

    #[cfg(windows)]
    fn begin() -> Self {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        // SAFETY: 플래그만 넘기는 Win32 호출이다. 화면 꺼짐(ES_DISPLAY_REQUIRED)은 막지 않는다
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        Self {}
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    fn begin() -> Self {
        Self {}
    }
}

#[cfg(windows)]
impl Drop for KeepAwake {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};
        // SAFETY: ES_CONTINUOUS만 넘겨 이 스레드가 걸어 둔 요구를 푼다
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[cfg(target_os = "macos")]
impl Drop for KeepAwake {
    fn drop(&mut self) {
        // SAFETY: beginActivityWithOptions_reason가 돌려준 객체를 그대로 넘긴다
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.activity) };
    }
}

/// 피크를 -50dB..0dB 구간에서 0..1로 바꾼다. 사람 귀에 가까운 로그 눈금이다
fn level(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db + 50.0) / 50.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer(path: &std::path::Path, rate: u32) -> Writer {
        hound::WavWriter::create(
            path,
            hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int },
        )
        .unwrap()
    }

    #[test]
    fn 데이터가_끊긴_트랙은_흐른_시간만큼_무음으로_채운다() {
        let dir = std::env::temp_dir().join(format!("sori-pad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut writers = vec![writer(&dir.join("a.wav"), 16_000), writer(&dir.join("b.wav"), 48_000)];
        // a는 3초 내내 받았고, b(루프백)는 1초만 받고 끊긴 상황
        let mut written = vec![48_000, 48_000];

        pad_gaps(&mut writers, &mut written, &[16_000, 48_000], Duration::from_secs(3)).unwrap();

        assert_eq!(written[0], 48_000);
        assert_eq!(written[1], 3 * 48_000 - 48_000 / 20);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 장치_버퍼_지연_정도는_채우지_않는다() {
        let dir = std::env::temp_dir().join(format!("sori-nopad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut writers = vec![writer(&dir.join("a.wav"), 16_000)];
        // 1초가 흘렀고 0.8초 분량만 도착(0.2초 지연)
        let mut written = vec![12_800];

        pad_gaps(&mut writers, &mut written, &[16_000], Duration::from_secs(1)).unwrap();

        assert_eq!(written[0], 12_800);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 무음은_레벨_0이다() {
        assert_eq!(level(0.0), 0.0);
    }

    #[test]
    fn 최대_진폭은_레벨_1이다() {
        assert_eq!(level(1.0), 1.0);
    }

    #[test]
    fn 영하_50데시벨_이하는_0으로_자른다() {
        assert_eq!(level(0.001), 0.0);
    }
}
