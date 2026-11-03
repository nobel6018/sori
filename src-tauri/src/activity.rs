//! 앱 밖 도구(MCP 서버)가 받아 적기와 요약의 진행을 볼 수 있게 데이터 폴더 `activity/<녹음 id>.json`에 적는다.
//! 화면은 이벤트로 받으므로 이 파일을 읽지 않는다. 기기 사이에 오갈 필요가 없어 동기화 대상(recordings/) 밖에 둔다.
//! 같은 단계에서 1% 미만으로 바뀐 값은 쓰지 않는다(화자 분리는 초당 수십 번 알린다).
//! @see ../../mcp/README.md

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// waiting, decoding, diarizing, transcribing, summarizing
    pub stage: String,
    /// 0~1. 요약처럼 진행률을 모르는 단계는 없음
    pub progress: Option<f32>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct Activity {
    dir: PathBuf,
    /// 녹음 id → 마지막으로 쓴 (단계, 퍼센트)
    last: Mutex<HashMap<String, (String, i32)>>,
}

impl Activity {
    /// 앱이 켜질 때 부른다. 앞서 비정상 종료로 남은 파일은 이미 끝난 일이라 지운다
    pub fn new(dir: PathBuf) -> Self {
        let _ = fs::remove_dir_all(&dir);
        Self { dir, last: Mutex::new(HashMap::new()) }
    }

    pub fn write(&self, id: &str, stage: &str, progress: Option<f32>) {
        let percent = progress.map_or(-1, |p| (p.clamp(0.0, 1.0) * 100.0) as i32);
        {
            let mut last = self.last.lock().unwrap();
            if last.get(id).is_some_and(|(s, p)| s == stage && *p == percent) {
                return;
            }
            last.insert(id.to_string(), (stage.to_string(), percent));
        }
        let entry = Entry { stage: stage.into(), progress, updated_at: chrono::Utc::now() };
        // 읽는 쪽이 반쯤 쓴 파일을 보지 않게 임시 파일에 쓰고 이름을 바꾼다. 진행 표시는 덤이라 실패해도 처리를 막지 않는다
        let _ = fs::create_dir_all(&self.dir).and_then(|_| {
            let path = self.dir.join(format!("{id}.json"));
            let tmp = self.dir.join(format!("{id}.{}.tmp", uuid::Uuid::new_v4().simple()));
            fs::write(&tmp, serde_json::to_vec(&entry).unwrap_or_default())?;
            fs::rename(&tmp, path)
        });
    }

    pub fn clear(&self, id: &str) {
        self.last.lock().unwrap().remove(id);
        let _ = fs::remove_file(self.dir.join(format!("{id}.json")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(dir: &std::path::Path, id: &str) -> Option<Entry> {
        serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).ok()?).ok()
    }

    #[test]
    fn 단계와_진행률을_적고_끝나면_지운다() {
        let dir = std::env::temp_dir().join(format!("sori-activity-{}", uuid::Uuid::new_v4()));
        let a = Activity::new(dir.clone());

        a.write("r1", "diarizing", Some(0.42));
        let e = read(&dir, "r1").unwrap();
        assert_eq!((e.stage.as_str(), e.progress), ("diarizing", Some(0.42)));

        a.write("r1", "summarizing", None);
        assert_eq!(read(&dir, "r1").unwrap().stage, "summarizing");

        a.clear("r1");
        assert!(read(&dir, "r1").is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 같은_단계에서_1퍼센트_미만_변화는_쓰지_않는다() {
        let dir = std::env::temp_dir().join(format!("sori-activity-{}", uuid::Uuid::new_v4()));
        let a = Activity::new(dir.clone());

        a.write("r1", "transcribing", Some(0.501));
        fs::remove_file(dir.join("r1.json")).unwrap();
        a.write("r1", "transcribing", Some(0.509));
        assert!(read(&dir, "r1").is_none());

        a.write("r1", "transcribing", Some(0.51));
        assert!(read(&dir, "r1").is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 켜질_때_남은_파일을_지운다() {
        let dir = std::env::temp_dir().join(format!("sori-activity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("old.json"), b"{}").unwrap();

        let _a = Activity::new(dir.clone());

        assert!(!dir.join("old.json").exists());
    }
}
