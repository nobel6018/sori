use std::fs::File;
use std::path::Path;

use eyre::{eyre, Result, WrapErr};
use rubato::{FftFixedIn, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// whisper.cpp와 sherpa-onnx 모델 모두 16kHz 모노 입력만 받는다
pub const SAMPLE_RATE: u32 = 16_000;

/// 오디오 파일을 디코딩해 16kHz 모노 f32 샘플로 돌려준다
pub fn load_mono_16k(path: &Path) -> Result<Vec<f32>> {
    let (samples, rate) = decode_to_mono(path)?;
    resample(&samples, rate, SAMPLE_RATE)
}

fn decode_to_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let file = File::open(path).wrap_err_with(|| format!("파일을 열 수 없습니다: {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .wrap_err("지원하지 않는 오디오 형식입니다")?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| eyre!("오디오 트랙이 없습니다"))?;
    let track_id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| eyre!("샘플레이트를 알 수 없습니다"))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .wrap_err("이 코덱은 디코딩할 수 없습니다")?;

    let mut mono = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // 손상된 프레임 하나 때문에 전체를 버리지 않는다
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let sb = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if sb.capacity() < decoded.capacity() * channels {
            *sb = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        sb.copy_interleaved_ref(decoded);

        for frame in sb.samples().chunks_exact(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }

    if mono.is_empty() {
        return Err(eyre!("디코딩된 오디오가 비어 있습니다"));
    }
    Ok((mono, rate))
}

fn resample(samples: &[f32], from: u32, to: u32) -> Result<Vec<f32>> {
    if from == to {
        return Ok(samples.to_vec());
    }
    const CHUNK: usize = 4096;
    let mut resampler = FftFixedIn::<f32>::new(from as usize, to as usize, CHUNK, 2, 1)?;
    let mut out = Vec::with_capacity(samples.len() * to as usize / from as usize + CHUNK);

    let mut pos = 0;
    while pos + CHUNK <= samples.len() {
        let block = resampler.process(&[&samples[pos..pos + CHUNK]], None)?;
        out.extend_from_slice(&block[0]);
        pos += CHUNK;
    }
    if pos < samples.len() {
        let block = resampler.process_partial(Some(&[&samples[pos..]]), None)?;
        out.extend_from_slice(&block[0]);
    }
    let tail = resampler.process_partial::<&[f32]>(None, None)?;
    out.extend_from_slice(&tail[0]);
    // FFT 리샘플러가 앞쪽에 넣는 지연만큼 잘라 타임스탬프를 원본과 맞춘다
    let delay = resampler.output_delay();
    let expected = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let end = (delay + expected).min(out.len());
    Ok(out[delay.min(end)..end].to_vec())
}

/// 트랙 하나를 조금씩 읽어 목표 샘플레이트로 바꿔 주는 통로
struct Lane {
    samples: hound::WavIntoSamples<std::io::BufReader<File>, i16>,
    resampler: Option<FftFixedIn<f32>>,
    pending: std::collections::VecDeque<f32>,
    /// FFT 리샘플러가 앞쪽에 넣는 지연. 이만큼 버려야 트랙끼리 시점이 맞는다
    skip: usize,
    /// 원본 길이에서 나와야 할 남은 출력 샘플 수. 리샘플러가 마지막 조각 뒤에 붙이는 무음을 잘라낸다
    left: u64,
    done: bool,
}

const MIX_CHUNK: usize = 4096;

impl Lane {
    fn open(path: &Path, target: u32) -> Result<Self> {
        let reader = hound::WavReader::open(path).wrap_err_with(|| format!("트랙을 열 수 없습니다: {}", path.display()))?;
        let rate = reader.spec().sample_rate;
        let left = reader.len() as u64 * target as u64 / rate as u64;
        let resampler = if rate == target {
            None
        } else {
            Some(FftFixedIn::<f32>::new(rate as usize, target as usize, MIX_CHUNK, 2, 1)?)
        };
        let skip = resampler.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            samples: reader.into_samples::<i16>(),
            resampler,
            pending: Default::default(),
            skip,
            left,
            done: false,
        })
    }

    fn fill(&mut self) -> Result<()> {
        let mut block = Vec::with_capacity(MIX_CHUNK);
        for s in self.samples.by_ref().take(MIX_CHUNK) {
            block.push(s? as f32 / 32768.0);
        }
        let last = block.len() < MIX_CHUNK;
        let out = match &mut self.resampler {
            None => block,
            Some(r) if !last => r.process(&[&block], None)?.remove(0),
            Some(r) => {
                let mut out = r.process_partial(Some(&[&block]), None)?.remove(0);
                out.extend(r.process_partial::<&[f32]>(None, None)?.remove(0));
                out
            }
        };
        let skipped = self.skip.min(out.len());
        self.skip -= skipped;
        let kept = (out.len() - skipped).min(self.left as usize);
        self.pending.extend(&out[skipped..skipped + kept]);
        self.left -= kept as u64;
        self.done = last;
        Ok(())
    }
}

/// 따로 녹음한 트랙(마이크, 시스템 소리)을 하나의 모노 WAV로 섞는다.
/// 1시간 녹음도 메모리에 다 올리지 않도록 트랙마다 조금씩 읽어 바로 쓴다. 섞은 길이(초)를 돌려준다
pub fn mix_wav_tracks(inputs: &[std::path::PathBuf], output: &Path) -> Result<f32> {
    let target = inputs
        .iter()
        .map(|p| Ok(hound::WavReader::open(p)?.spec().sample_rate))
        .collect::<Result<Vec<u32>>>()?
        .into_iter()
        .max()
        .ok_or_else(|| eyre!("섞을 트랙이 없습니다"))?;
    let mut lanes = inputs.iter().map(|p| Lane::open(p, target)).collect::<Result<Vec<_>>>()?;

    let mut writer = hound::WavWriter::create(
        output,
        hound::WavSpec {
            channels: 1,
            sample_rate: target,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    let mut written: u64 = 0;

    loop {
        for lane in &mut lanes {
            while !lane.done && lane.pending.len() < MIX_CHUNK {
                lane.fill()?;
            }
        }
        // 아직 읽을 게 남은 트랙은 가진 만큼만, 끝난 트랙은 무음으로 채워 길이를 맞춘다
        let n = if lanes.iter().all(|l| l.done) {
            lanes.iter().map(|l| l.pending.len()).max().unwrap_or(0)
        } else {
            lanes.iter().filter(|l| !l.done).map(|l| l.pending.len()).min().unwrap_or(0)
        };
        if n == 0 && lanes.iter().all(|l| l.done) {
            break;
        }
        for _ in 0..n {
            let sum: f32 = lanes.iter_mut().map(|l| l.pending.pop_front().unwrap_or(0.0)).sum();
            writer.write_sample((sum.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        written += n as u64;
    }
    writer.finalize()?;
    Ok(written as f32 / target as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tone(path: &Path, rate: u32, secs: f32, amp: f32) {
        let mut w = hound::WavWriter::create(
            path,
            hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int },
        )
        .unwrap();
        for i in 0..(rate as f32 * secs) as usize {
            let s = amp * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin();
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn 샘플레이트가_다른_두_트랙을_높은_쪽으로_섞는다() {
        let dir = std::env::temp_dir().join(format!("sori-mix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, out) = (dir.join("a.wav"), dir.join("b.wav"), dir.join("out.wav"));
        write_tone(&a, 24_000, 3.0, 0.3);
        write_tone(&b, 48_000, 2.0, 0.3);

        let duration = mix_wav_tracks(&[a, b], &out).unwrap();

        let spec = hound::WavReader::open(&out).unwrap().spec();
        assert_eq!(spec.sample_rate, 48_000);
        // 긴 트랙 길이에 맞추고, 리샘플링 오차는 10ms 안쪽이어야 한다
        assert!((duration - 3.0).abs() < 0.01, "duration = {duration}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 트랙이_하나면_그대로_복사한다() {
        let dir = std::env::temp_dir().join(format!("sori-mix1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, out) = (dir.join("a.wav"), dir.join("out.wav"));
        write_tone(&a, 16_000, 1.5, 0.5);

        let duration = mix_wav_tracks(&[a], &out).unwrap();

        assert!((duration - 1.5).abs() < 0.001);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
