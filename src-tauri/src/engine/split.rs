//! 화자를 나누지 않을 때 녹음을 받아 적을 조각으로 자른다. whisper 입력은 30초까지라,
//! 30초마다 기계적으로 자르면 말하는 도중에 끊긴다. 창 끝 몇 초에서 가장 조용한 곳을 찾아 자른다

use super::audio::SAMPLE_RATE;
use super::diarize::SpeakerTurn;

/// 한 조각의 최대 길이. transcribe.rs MAX_CHUNK_SECS와 같다
const MAX_SECS: f32 = 30.0;
/// 창의 마지막 이만큼에서 자를 곳을 찾는다
const SEARCH_SECS: f32 = 8.0;
/// 소리 크기를 재는 단위
const FRAME_SECS: f32 = 0.1;
/// 조각 안에서 가장 큰 소리가 이보다 작으면(약 -46dBFS) 말이 없는 것으로 보고 건너뛴다.
/// 무음을 whisper에 넣으면 "시청해 주셔서 감사합니다" 같은 환각이 나온다
const SILENT_PEAK_RMS: f32 = 0.005;

/// 녹음 전체를 화자 한 명(0)의 구간들로 자른다
pub fn split_by_silence(samples: &[f32]) -> Vec<SpeakerTurn> {
    let frame = (FRAME_SECS * SAMPLE_RATE as f32) as usize;
    let rms: Vec<f32> = samples
        .chunks(frame)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len().max(1) as f32).sqrt())
        .collect();
    let n = rms.len();
    let max_frames = (MAX_SECS / FRAME_SECS) as usize;
    let search = (SEARCH_SECS / FRAME_SECS) as usize;

    let mut turns = Vec::new();
    let mut start = 0;
    while start < n {
        let end = if n - start <= max_frames {
            n
        } else {
            // 창 끝 8초 안에서 가장 조용한 프레임에서 자른다
            let from = start + max_frames - search;
            let to = start + max_frames;
            (from..to).min_by(|&a, &b| rms[a].total_cmp(&rms[b])).unwrap_or(to) + 1
        };
        let peak = rms[start..end].iter().copied().fold(0.0f32, f32::max);
        if peak >= SILENT_PEAK_RMS {
            turns.push(SpeakerTurn {
                start: start as f32 * FRAME_SECS,
                end: (end as f32 * FRAME_SECS).min(samples.len() as f32 / SAMPLE_RATE as f32),
                speaker: 0,
            });
        }
        start = end;
    }
    turns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * SAMPLE_RATE as f32) as usize).map(|i| amp * (i as f32 * 0.05).sin()).collect()
    }

    #[test]
    fn 창_끝의_조용한_곳에서_자른다() {
        // 25초 말, 1초 쉼, 20초 말
        let mut s = tone(25.0, 0.3);
        s.extend(tone(1.0, 0.0));
        s.extend(tone(20.0, 0.3));

        let turns = split_by_silence(&s);

        assert_eq!(turns.len(), 2);
        assert!((25.0..=26.1).contains(&turns[0].end), "{:?}", turns);
        assert!(turns.iter().all(|t| t.end - t.start <= MAX_SECS + 0.01));
    }

    #[test]
    fn 말이_없는_조각은_건너뛴다() {
        let mut s = tone(10.0, 0.3);
        s.extend(tone(40.0, 0.0));

        let turns = split_by_silence(&s);

        assert!(turns.iter().all(|t| t.start < 30.0), "{:?}", turns);
    }
}
