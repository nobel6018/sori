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
