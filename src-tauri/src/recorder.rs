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
    let setup = || -> Result<(Vec<cpal::Stream>, Vec<Writer>)> {
        let mut streams = Vec::new();
        let mut writers = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            let (stream, rate) = open_stream(&track.source, index, tx.clone())?;
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
        Ok((streams, writers))
    };

    let (streams, mut writers) = match setup() {
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
    let mut written = vec![0u64; writers.len()];
    let mut peaks = vec![0.0f32; writers.len()];
    let mut last_flush = Instant::now();
    let mut last_level = Instant::now();
    let mut interrupted = None;

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Samples(i, samples)) => {
                write_samples(&mut writers[i], &samples)?;
                peaks[i] = samples.iter().fold(peaks[i], |p, s| p.max(s.abs()));
                written[i] += samples.len() as u64;
            }
            Ok(Event::StreamError(e)) => {
                interrupted = Some(e);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
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

    // 스트림을 먼저 멈춘 뒤 채널에 남은 샘플까지 받아 적는다
    drop(streams);
    while let Ok(event) = rx.try_recv() {
        if let Event::Samples(i, samples) = event {
            write_samples(&mut writers[i], &samples)?;
            written[i] += samples.len() as u64;
        }
    }
    for w in writers {
        w.finalize()?;
    }
    on_level(vec![0.0; written.len()]);
    // 사용자가 멈추기를 누르지 않았는데 끝났으면 화면에 알려 멈추기 처리를 하게 한다
    if let Some(reason) = &interrupted {
        on_interrupted(reason.clone());
    }

    Ok(Finished { interrupted })
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
