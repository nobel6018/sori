use std::path::{Path, PathBuf};
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

enum Event {
    Samples(Vec<f32>),
    StreamError(String),
}

/// 녹음 중인 스레드의 핸들. stop을 부르거나 drop하면 녹음을 끝내고 파일을 마무리한다
pub struct ActiveRecording {
    stop_tx: mpsc::Sender<()>,
    handle: Option<JoinHandle<Result<Finished>>>,
}

pub struct Finished {
    pub duration: f32,
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

/// 장치 기본 샘플레이트 그대로 16비트 모노 WAV로 녹음한다.
/// 리샘플링은 전사할 때 한 번만 하면 되므로 녹음 중에는 하지 않는다
pub fn start(
    device_id: Option<&str>,
    path: &Path,
    on_level: impl Fn(f32) + Send + 'static,
    on_interrupted: impl FnOnce(String) + Send + 'static,
) -> Result<ActiveRecording> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    let device_id = device_id.map(str::to_string);
    let path = path.to_path_buf();

    // cpal 스트림은 macOS에서 다른 스레드로 옮길 수 없어서, 만들고 끝낼 때까지 한 스레드가 소유한다
    let handle = std::thread::spawn(move || run(device_id, path, stop_rx, ready_tx, on_level, on_interrupted));

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
    device_id: Option<String>,
    path: PathBuf,
    stop_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::Sender<Result<()>>,
    on_level: impl Fn(f32),
    on_interrupted: impl FnOnce(String),
) -> Result<Finished> {
    let (tx, rx) = mpsc::channel::<Event>();
    let setup = || -> Result<(cpal::Stream, hound::WavWriter<std::io::BufWriter<std::fs::File>>, u32)> {
        let device = open_device(device_id.as_deref())?;
        let config = device.default_input_config().wrap_err("입력 장치 설정을 읽지 못했습니다")?;
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config.into(), channels, tx.clone())?,
            SampleFormat::I16 => build::<i16>(&device, config.into(), channels, tx.clone())?,
            SampleFormat::I32 => build::<i32>(&device, config.into(), channels, tx.clone())?,
            SampleFormat::U16 => build::<u16>(&device, config.into(), channels, tx.clone())?,
            other => return Err(eyre!("지원하지 않는 샘플 형식입니다: {other}")),
        };
        let writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        stream.play().wrap_err("녹음을 시작하지 못했습니다. 마이크 권한을 확인해 주세요")?;
        Ok((stream, writer, rate))
    };

    let (stream, mut writer, rate) = match setup() {
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
    let mut written: u64 = 0;
    let mut last_flush = Instant::now();
    let mut last_level = Instant::now();
    let mut peak = 0.0f32;
    let mut interrupted = None;

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Samples(samples)) => {
                for s in &samples {
                    writer.write_sample(i16::from_sample(s.clamp(-1.0, 1.0)))?;
                    peak = peak.max(s.abs());
                }
                written += samples.len() as u64;
            }
            Ok(Event::StreamError(e)) => {
                interrupted = Some(e);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_level.elapsed() >= LEVEL_INTERVAL {
            on_level(level(peak));
            peak = 0.0;
            last_level = Instant::now();
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            writer.flush()?;
            last_flush = Instant::now();
        }
    }

    // 스트림을 먼저 멈춘 뒤 채널에 남은 샘플까지 받아 적는다
    drop(stream);
    while let Ok(Event::Samples(samples)) = rx.try_recv() {
        for s in &samples {
            writer.write_sample(i16::from_sample(s.clamp(-1.0, 1.0)))?;
        }
        written += samples.len() as u64;
    }
    writer.finalize()?;
    on_level(0.0);
    // 사용자가 멈추기를 누르지 않았는데 끝났으면 화면에 알려 멈추기 처리를 하게 한다
    if let Some(reason) = &interrupted {
        on_interrupted(reason.clone());
    }

    Ok(Finished {
        duration: written as f32 / rate as f32,
        interrupted,
    })
}

fn open_device(device_id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let device = match device_id {
        Some(id) => {
            let id = id.parse().map_err(|e| eyre!("잘못된 장치 id입니다: {e}"))?;
            host.device_by_id(&id)
        }
        None => host.default_input_device(),
    };
    device.ok_or_else(|| eyre!("마이크를 찾을 수 없습니다"))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
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
            let _ = tx.send(Event::Samples(mono));
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

/// 녹음하는 동안 macOS App Nap과 잠자기를 막는다. 회의 중에는 앱이 대부분 뒤에 가려져 있는데,
/// App Nap에 들어가면 녹음 스레드가 느려져 버퍼 오버런으로 소리가 빠진다
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

    #[cfg(not(target_os = "macos"))]
    fn begin() -> Self {
        Self {}
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
