//! iPhone 저장소. 녹음마다 폴더 하나에 meta.json, audio.m4a, transcript.json 등을 둔다(데스크톱 store.rs와 같은 구조).
//! 전사와 요약은 데스크톱이 만들어 동기화로 오고, 이 앱은 읽기만 해서 meta.json은 JSON 값 그대로 다룬다.
//! @see docs/mobile.md

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use eyre::{eyre, Result, WrapErr};
use serde_json::{json, Value};

pub struct Store {
    root: PathBuf,
    data: PathBuf,
    /// meta.json을 읽고 고쳐 쓰는 동안 동기화와 화면이 서로 덮어쓰지 않게 한다
    meta_lock: Mutex<()>,
    create_lock: Mutex<()>,
}

impl Store {
    pub fn new(data: PathBuf) -> Result<Self> {
        let root = data.join("recordings");
        fs::create_dir_all(&root)?;
        Ok(Self { root, data, meta_lock: Mutex::new(()), create_lock: Mutex::new(()) })
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }

    pub fn dir(&self, id: &str) -> Result<PathBuf> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(eyre!("잘못된 녹음 id입니다: {id}"));
        }
        Ok(self.root.join(id))
    }

    pub fn ids(&self) -> Result<Vec<String>> {
        Ok(fs::read_dir(&self.root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("meta.json").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect())
    }

    /// 최근 것부터
    pub fn list(&self) -> Result<Vec<Value>> {
        let mut all: Vec<Value> = self.ids()?.iter().filter_map(|id| self.read(id).ok()).collect();
        all.sort_by(|a, b| b["createdAt"].as_str().unwrap_or_default().cmp(a["createdAt"].as_str().unwrap_or_default()));
        Ok(all)
    }

    pub fn read(&self, id: &str) -> Result<Value> {
        read_json(&self.dir(id)?.join("meta.json"))
    }

    pub fn update(&self, id: &str, change: &mut dyn FnMut(&mut Value)) -> Result<Value> {
        let _guard = self.meta_lock.lock().unwrap();
        let path = self.dir(id)?.join("meta.json");
        let mut meta: Value = read_json(&path).wrap_err_with(|| format!("녹음을 찾을 수 없습니다: {id}"))?;
        change(&mut meta);
        write_json(&path, &meta)?;
        Ok(meta)
    }

    pub fn create_raw(&self, id: &str, meta: Value) -> Result<()> {
        let _guard = self.create_lock.lock().unwrap();
        let dir = self.dir(id)?;
        fs::create_dir_all(&dir)?;
        write_json(&dir.join("meta.json"), &meta)
    }

    /// 이 기기에서 새로 녹음할 항목. 번호는 남은 녹음의 가장 큰 번호 + 1이고(맨 뒤를 지우면 이어 쓴다),
    /// 동기화할 때 겹치면 엔진이 고친다. 데스크톱 store.rs next_number와 같은 규칙. @see docs/sync.md
    pub fn create_recording(&self, title: &str) -> Result<Value> {
        let _guard = self.create_lock.lock().unwrap();
        let number = self.list()?.iter().filter_map(|m| m["number"].as_u64()).max().unwrap_or(0) + 1;

        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let meta = json!({
            "id": id,
            "number": number,
            "title": title,
            "createdAt": now,
            "updatedAt": now,
            "sourceName": "iPhone",
            "audioFile": "audio.m4a",
            "status": "recording",
            "notes": [],
            "speakerNames": {},
            "languages": [],
        });
        let dir = self.dir(&id)?;
        fs::create_dir_all(&dir)?;
        write_json(&dir.join("meta.json"), &meta)?;
        Ok(meta)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        fs::remove_dir_all(self.dir(id)?)?;
        Ok(())
    }

    pub fn transcript(&self, id: &str) -> Result<Option<Value>> {
        let path = self.dir(id)?.join("transcript.json");
        if !path.is_file() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// iPhone에서 바로 만든 미리 전사(preview-transcript.json). 이 기기에만 두고 동기화하지 않는다(docs/organize.md)
    pub fn preview_transcript(&self, id: &str) -> Result<Option<Value>> {
        let path = self.dir(id)?.join("preview-transcript.json");
        if !path.is_file() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn save_preview_transcript(&self, id: &str, transcript: &Value) -> Result<()> {
        write_json(&self.dir(id)?.join("preview-transcript.json"), transcript)
    }

    /// 오래된 것부터. summaries.json이 없으면(데스크톱 0.3.0 이하) summary.json 하나를 목록으로 본다
    pub fn summaries(&self, id: &str) -> Result<Vec<Value>> {
        let dir = self.dir(id)?;
        if let Ok(all) = read_json::<Vec<Value>>(&dir.join("summaries.json")) {
            return Ok(all);
        }
        Ok(read_json::<Value>(&dir.join("summary.json")).ok().into_iter().collect())
    }

    pub fn audio_path(&self, meta: &Value) -> Result<PathBuf> {
        let id = meta["id"].as_str().unwrap_or_default();
        let file = meta["audioFile"].as_str().unwrap_or("audio.m4a");
        if file.contains('/') || file.contains("..") {
            return Err(eyre!("잘못된 오디오 파일 이름입니다: {file}"));
        }
        Ok(self.dir(id)?.join(file))
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("sori-mobile-test-{}", uuid::Uuid::new_v4().simple()));
        Store::new(dir).unwrap()
    }

    #[test]
    fn 새_녹음은_번호를_이어서_매기고_최근_것부터_나온다() {
        let s = store();
        let a = s.create_recording("첫째").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = s.create_recording("둘째").unwrap();

        assert_eq!((a["number"].as_u64(), b["number"].as_u64()), (Some(1), Some(2)));
        let titles: Vec<String> = s.list().unwrap().iter().map(|m| m["title"].as_str().unwrap().to_string()).collect();
        assert_eq!(titles, vec!["둘째", "첫째"]);
    }

    #[test]
    fn 맨_뒤_녹음을_지우면_그_번호부터_이어_쓴다() {
        let s = store();
        s.create_recording("a").unwrap();
        let b = s.create_recording("b").unwrap();
        s.delete(b["id"].as_str().unwrap()).unwrap();

        let c = s.create_recording("c").unwrap();

        assert_eq!(c["number"].as_u64(), Some(2));
    }

    #[test]
    fn 중간_녹음을_지우면_빈칸으로_두고_남은_번호는_그대로다() {
        let s = store();
        let a = s.create_recording("a").unwrap();
        s.create_recording("b").unwrap();
        s.delete(a["id"].as_str().unwrap()).unwrap();

        let c = s.create_recording("c").unwrap();

        assert_eq!(c["number"].as_u64(), Some(3));
    }
}
