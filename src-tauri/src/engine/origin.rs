use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::audio::SAMPLE_RATE;
use super::diarize::SpeakerTurn;

/// 화자가 이 컴퓨터 마이크 앞에 있었는지, 시스템 소리(화상 회의 상대)로 들어왔는지
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Local,
    Remote,
}

/// 화자 발화 에너지 중 시스템 트랙 비중이 이보다 크면 원격, 이보다 작으면(LOCAL_BELOW) 이쪽으로 본다.
/// 스피커로 들으면 상대 목소리가 마이크에도 들어가 원격 화자도 시스템 비중이 0.5~0.9 정도로 낮아진다.
/// 반대로 이쪽 목소리는 시스템 트랙에 거의 들어가지 않는다
const REMOTE_ABOVE: f32 = 0.35;
const LOCAL_BELOW: f32 = 0.1;

/// 화자별로 말한 구간의 마이크와 시스템 트랙 에너지를 비교해 어느 쪽 사람인지 정한다.
/// 둘 사이에 걸치면(같은 방에서 스피커폰으로 섞이는 경우 등) 판단하지 않는다
pub fn classify(turns: &[SpeakerTurn], mic: &[f32], system: &[f32]) -> BTreeMap<i32, Origin> {
    let mut energy: BTreeMap<i32, (f64, f64)> = BTreeMap::new();
    for t in turns {
        let from = (t.start * SAMPLE_RATE as f32) as usize;
        let to = (t.end * SAMPLE_RATE as f32) as usize;
        let e = energy.entry(t.speaker).or_default();
        e.0 += sum_squares(mic, from, to);
        e.1 += sum_squares(system, from, to);
    }
    energy
        .into_iter()
        .filter_map(|(speaker, (mic, sys))| {
            let total = mic + sys;
            if total <= f64::EPSILON {
                return None;
            }
            let share = (sys / total) as f32;
            if share > REMOTE_ABOVE {
                Some((speaker, Origin::Remote))
            } else if share < LOCAL_BELOW {
                Some((speaker, Origin::Local))
            } else {
                None
            }
        })
        .collect()
}

fn sum_squares(samples: &[f32], from: usize, to: usize) -> f64 {
    let to = to.min(samples.len());
    let from = from.min(to);
    samples[from..to].iter().map(|s| (*s as f64) * (*s as f64)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f32, end: f32, speaker: i32) -> SpeakerTurn {
        SpeakerTurn { start, end, speaker }
    }

    /// 0..2초는 마이크에만 소리, 2..4초는 시스템에 크게 마이크에 작게(스피커 반향)
    fn tracks() -> (Vec<f32>, Vec<f32>) {
        let n = SAMPLE_RATE as usize;
        let mut mic = vec![0.0; n * 4];
        let mut sys = vec![0.0; n * 4];
        mic[..n * 2].iter_mut().for_each(|s| *s = 0.5);
        mic[n * 2..].iter_mut().for_each(|s| *s = 0.2);
        sys[n * 2..].iter_mut().for_each(|s| *s = 0.5);
        (mic, sys)
    }

    #[test]
    fn 마이크에만_들어온_화자는_이쪽이다() {
        let (mic, sys) = tracks();

        let origins = classify(&[turn(0.0, 2.0, 0)], &mic, &sys);

        assert_eq!(origins.get(&0), Some(&Origin::Local));
    }

    #[test]
    fn 스피커_반향이_있어도_시스템_비중이_크면_원격이다() {
        let (mic, sys) = tracks();

        let origins = classify(&[turn(2.0, 4.0, 1)], &mic, &sys);

        assert_eq!(origins.get(&1), Some(&Origin::Remote));
    }

    #[test]
    fn 양쪽에_애매하게_걸친_화자는_판단하지_않는다() {
        // 같은 방 스피커폰처럼 시스템 쪽에도 작게 들어온 경우. 시스템 비중 약 0.14
        let n = SAMPLE_RATE as usize;
        let mic = vec![0.5; n * 2];
        let sys = vec![0.2; n * 2];

        let origins = classify(&[turn(0.0, 2.0, 2)], &mic, &sys);

        assert_eq!(origins.get(&2), None);
    }
}
