use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eyre::Result;
use serde::{Deserialize, Serialize};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

use super::audio::SAMPLE_RATE;
use super::diarize::SpeakerTurn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Utterance {
    pub start: f32,
    pub end: f32,
    pub speaker: i32,
    pub text: String,
    pub lang: String,
}

/// whisper 입력 창이 30초라 조각을 그보다 길게 만들지 않는다
const MAX_CHUNK_SECS: f32 = 30.0;
/// 같은 화자의 구간 사이 침묵이 이보다 짧으면 한 조각으로 묶는다
const MERGE_GAP_SECS: f32 = 1.0;
/// 이보다 짧은 조각은 언어 감지가 자주 틀려 "Thank you." 같은 환각이 나온다
const MIN_DETECT_SECS: f32 = 2.0;
/// 이보다 짧은 조각은 전사하지 않는다. 1초 미만을 무음으로 채워 돌리면 "뉴스였습니다" 같은 환각이 나왔다
const MIN_CHUNK_SECS: f32 = 0.8;
/// 전체 발화 시간 중 이 비율보다 적게 감지된 언어는 오감지로 본다
const MINOR_LANG_RATIO: f32 = 0.1;

/// 무음이나 잡음에서 whisper가 습관처럼 내놓는 문장들
const HALLUCINATIONS: &[&str] = &[
    "thank you.",
    "thanks for watching!",
    "시청해주셔서 감사합니다.",
    "구독과 좋아요 부탁드립니다.",
    "mbc 뉴스",
    "请不吝点赞 订阅 转发 打赏支持明镜与点点栏目",
    "字幕由amara.org社区提供",
    "感谢观看",
    "谢谢观看",
    "感謝觀看",
];

#[derive(Debug, Clone)]
struct Chunk {
    start: f32,
    end: f32,
    speaker: i32,
}

impl Chunk {
    fn duration(&self) -> f32 {
        self.end - self.start
    }
}

pub struct TranscribeOptions {
    /// 녹음에 쓰인 언어. 비우면 조각마다 감지하고, 하나면 고정하고,
    /// 여럿이면 감지 결과가 목록 밖일 때 목록의 언어로만 다시 돌린다
    pub languages: Vec<String>,
    pub threads: i32,
}

/// 화자 구간을 30초 이내 조각으로 묶어 조각마다 전사한다.
/// 언어를 고정하고 파일 전체를 한 번에 돌리면 긴 회의에서 같은 문장이 반복되는 환각이 났다
pub fn transcribe(
    ctx: &WhisperContext,
    samples: &[f32],
    turns: &[SpeakerTurn],
    options: &TranscribeOptions,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(f32),
) -> Result<Vec<Utterance>> {
    let chunks: Vec<Chunk> = build_chunks(turns)
        .into_iter()
        .filter(|c| c.duration() >= MIN_CHUNK_SECS)
        .collect();
    let total: f32 = chunks.iter().map(Chunk::duration).sum::<f32>().max(f32::EPSILON);
    let mut state = ctx.create_state()?;

    let mut results: Vec<Option<(String, Vec<Utterance>)>> = vec![None; chunks.len()];
    let mut done = 0.0;

    let fixed = match options.languages.as_slice() {
        [only] => Some(only.as_str()),
        _ => None,
    };
    let first_pass: Vec<usize> = (0..chunks.len())
        .filter(|&i| fixed.is_some() || chunks[i].duration() >= MIN_DETECT_SECS)
        .collect();

    for &i in &first_pass {
        check_cancel(&cancel)?;
        results[i] = Some(run_chunk(&mut state, samples, &chunks[i], fixed.unwrap_or("auto"), options.threads, &cancel)?);
        done += chunks[i].duration();
        on_progress(done / total);
    }

    if fixed.is_none() {
        let detected = major_languages(&chunks, &results);
        let major = if options.languages.is_empty() {
            detected
        } else {
            // 짧은 조각에 쓸 주 언어가 맨 앞에 오도록, 지정 언어를 실제 감지된 비중 순으로 정렬한다
            let mut langs = options.languages.clone();
            langs.sort_by_key(|l| detected.iter().position(|d| d == l).unwrap_or(usize::MAX));
            langs
        };
        let dominant = major.first().cloned().unwrap_or_else(|| "en".to_string());

        for i in 0..chunks.len() {
            check_cancel(&cancel)?;
            match &results[i] {
                // 짧은 조각은 감지하지 않고 회의 주 언어로 돌린다
                None => {
                    results[i] = Some(run_chunk(&mut state, samples, &chunks[i], &dominant, options.threads, &cancel)?);
                    done += chunks[i].duration();
                    on_progress(done / total);
                }
                // 소수 언어로 감지된 조각은 주요 언어마다 다시 돌려 가장 확신 높은 결과를 쓴다
                Some((lang, _)) if !major.contains(lang) => {
                    let mut best: Option<(f32, (String, Vec<Utterance>))> = None;
                    for candidate in &major {
                        let (lang, utts, score) =
                            run_chunk_scored(&mut state, samples, &chunks[i], candidate, options.threads, &cancel)?;
                        if best.as_ref().is_none_or(|(s, _)| score > *s) {
                            best = Some((score, (lang, utts)));
                        }
                    }
                    if let Some((_, r)) = best {
                        results[i] = Some(r);
                    }
                }
                Some(_) => {}
            }
        }
    }

    let mut utterances: Vec<Utterance> = results
        .into_iter()
        .flatten()
        .flat_map(|(_, u)| u)
        .filter(|u| !is_hallucination(&u.text))
        .map(|mut u| {
            // 대화체 한국어에서 whisper가 자막처럼 앞에 붙이는 대시를 떼어낸다
            u.text = u.text.trim_start_matches("- ").to_string();
            u
        })
        .collect();
    utterances.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(utterances)
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        eyre::bail!("취소했습니다");
    }
    Ok(())
}

fn build_chunks(turns: &[SpeakerTurn]) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    for t in turns {
        let mut start = t.start;
        while start < t.end {
            let end = (start + MAX_CHUNK_SECS).min(t.end);
            match chunks.last_mut() {
                Some(last)
                    if last.speaker == t.speaker
                        && start - last.end < MERGE_GAP_SECS
                        && end - last.start <= MAX_CHUNK_SECS =>
                {
                    last.end = end;
                }
                _ => chunks.push(Chunk {
                    start,
                    end,
                    speaker: t.speaker,
                }),
            }
            start = end;
        }
    }
    chunks
}

fn major_languages(chunks: &[Chunk], results: &[Option<(String, Vec<Utterance>)>]) -> Vec<String> {
    let mut by_lang: HashMap<&str, f32> = HashMap::new();
    for (chunk, result) in chunks.iter().zip(results) {
        if let Some((lang, _)) = result {
            *by_lang.entry(lang.as_str()).or_default() += chunk.duration();
        }
    }
    let total: f32 = by_lang.values().sum();
    let mut langs: Vec<(&str, f32)> = by_lang
        .into_iter()
        .filter(|(_, secs)| *secs >= total * MINOR_LANG_RATIO)
        .collect();
    langs.sort_by(|a, b| b.1.total_cmp(&a.1));
    langs.into_iter().map(|(l, _)| l.to_string()).collect()
}

fn run_chunk(
    state: &mut whisper_rs::WhisperState,
    samples: &[f32],
    chunk: &Chunk,
    language: &str,
    threads: i32,
    cancel: &Arc<AtomicBool>,
) -> Result<(String, Vec<Utterance>)> {
    let (lang, utts, _) = run_chunk_scored(state, samples, chunk, language, threads, cancel)?;
    Ok((lang, utts))
}

/// 점수는 토큰 확률의 평균이다. 같은 조각을 여러 언어로 돌렸을 때 비교하는 데만 쓴다
fn run_chunk_scored(
    state: &mut whisper_rs::WhisperState,
    samples: &[f32],
    chunk: &Chunk,
    language: &str,
    threads: i32,
    cancel: &Arc<AtomicBool>,
) -> Result<(String, Vec<Utterance>, f32)> {
    let from = ((chunk.start * SAMPLE_RATE as f32) as usize).min(samples.len());
    let to = ((chunk.end * SAMPLE_RATE as f32) as usize).min(samples.len());
    let mut audio = samples[from..to].to_vec();
    // whisper.cpp는 1초 미만 입력을 거부하므로 무음으로 채운다
    let min_len = SAMPLE_RATE as usize + SAMPLE_RATE as usize / 10;
    if audio.len() < min_len {
        audio.resize(min_len, 0.0);
    }

    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: -1.0,
    });
    params.set_language(Some(language));
    params.set_n_threads(threads);
    params.set_no_context(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    // whisper-rs 0.16의 set_abort_callback_safe는 클로저를 Box<dyn>으로 감싼 뒤 구체 타입으로
    // 되읽어서 쓰레기 값을 읽고 인코더를 임의로 중단시킨다. 원시 API로 AtomicBool을 직접 넘긴다
    unsafe extern "C" fn should_abort(user_data: *mut std::ffi::c_void) -> bool {
        unsafe { (*(user_data as *const AtomicBool)).load(Ordering::Relaxed) }
    }
    // SAFETY: cancel은 이 함수가 끝날 때까지 살아 있고, state.full은 반환 전에 콜백 호출을 마친다
    unsafe {
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data(Arc::as_ptr(cancel) as *mut std::ffi::c_void);
    }

    state.full(params, &audio)?;

    let lang = whisper_rs::get_lang_str(state.full_lang_id_from_state())
        .unwrap_or(language)
        .to_string();

    let mut utterances = Vec::new();
    let mut prob_sum = 0.0;
    let mut prob_count = 0;
    for seg in state.as_iter() {
        let text = seg.to_str_lossy()?.trim().to_string();
        for t in 0..seg.n_tokens() {
            if let Some(token) = seg.get_token(t) {
                prob_sum += token.token_probability();
                prob_count += 1;
            }
        }
        if text.is_empty() {
            continue;
        }
        // 타임스탬프 단위는 10ms
        let start = chunk.start + seg.start_timestamp() as f32 / 100.0;
        let end = (chunk.start + seg.end_timestamp() as f32 / 100.0).min(chunk.end);
        utterances.push(Utterance {
            start,
            end: end.max(start),
            speaker: chunk.speaker,
            text,
            lang: lang.clone(),
        });
    }
    let score = if prob_count > 0 { prob_sum / prob_count as f32 } else { 0.0 };
    Ok((lang, utterances, score))
}

fn is_hallucination(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    HALLUCINATIONS.iter().any(|h| normalized == *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f32, end: f32, speaker: i32) -> SpeakerTurn {
        SpeakerTurn { start, end, speaker }
    }

    #[test]
    fn 같은_화자의_가까운_구간은_한_조각으로_묶는다() {
        let chunks = build_chunks(&[turn(0.0, 3.0, 0), turn(3.5, 6.0, 0)]);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].end, 6.0);
    }

    #[test]
    fn 화자가_바뀌면_조각을_나눈다() {
        let chunks = build_chunks(&[turn(0.0, 3.0, 0), turn(3.2, 6.0, 1)]);

        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn 긴_구간은_30초_단위로_자른다() {
        let chunks = build_chunks(&[turn(0.0, 75.0, 0)]);

        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.duration() <= MAX_CHUNK_SECS));
    }

    #[test]
    fn 묶었을_때_30초를_넘으면_새_조각을_만든다() {
        let chunks = build_chunks(&[turn(0.0, 20.0, 0), turn(20.5, 35.0, 0)]);

        assert_eq!(chunks.len(), 2);
    }
}
