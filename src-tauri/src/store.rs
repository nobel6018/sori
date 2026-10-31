use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::actions::Shortcuts;
use crate::engine::{ModelPaths, Transcript};
use crate::summarize::{Provider, Summary, SummaryTemplate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Recording,
    New,
    Processing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    /// 사람이 부르기 쉬운 번호(#12). 만든 순서대로 1부터 붙고 지워도 다시 쓰지 않는다. 0.4.3 이하 녹음은 시작할 때 채운다
    #[serde(default)]
    pub number: u32,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub source_name: String,
    /// 녹음 폴더 안의 파일 이름. 원본을 복사해 두므로 원본을 지워도 재생된다
    pub audio_file: String,
    pub status: Status,
    pub error: Option<String>,
    pub duration: Option<f32>,
    /// 화자 번호(문자열) → 사용자가 붙인 이름
    #[serde(default)]
    pub speaker_names: BTreeMap<String, String>,
    #[serde(default)]
    pub num_speakers: Option<i32>,
    #[serde(default)]
    pub languages: Vec<String>,
    /// 앱에서 녹음한 경우 소리별 트랙 파일 이름(mic.wav, system.wav). 섞은 결과가 audio_file이다
    #[serde(default)]
    pub tracks: Vec<String>,
    /// 녹음하며 남긴 메모와 북마크(글이 빈 메모). 시각순
    #[serde(default)]
    pub notes: Vec<Note>,
    /// 화자 번호(문자열) → 기억한 목소리 id. 이름을 목소리로 알아봐 붙인 화자만 있다
    #[serde(default)]
    pub voice_ids: BTreeMap<String, String>,
    /// 마지막으로 처리할 때 화자를 나눴는지. 다시 처리할 때 같은 방식을 미리 고른다
    #[serde(default = "yes")]
    pub diarize: bool,
    /// 기기끼리 나누는 필드(제목, 화자 이름 등)를 마지막으로 고친 시각. 두 기기가 함께 고쳤을 때 늦은 쪽을 고른다
    /// @see docs/sync.md
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    /// `/`로 계층을 만든 태그(업무/영업). sori_sync::tags로 정리한다
    /// @see docs/organize.md
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 사람이 태그를 고친 시각. 있으면 AI 자동 분류가 덮어쓰지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags_edited_at: Option<DateTime<Utc>>,
    /// AI가 태그를 붙인 시각. 한 번 붙였으면 다시 하지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_tagged_at: Option<DateTime<Utc>>,
    /// 녹음을 시작한 곳. 설정에서 끄면 남기지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

/// 녹음을 시작한 곳. iPhone은 GPS, Mac은 Wi-Fi로 받는다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    /// 오차 반경(m)
    #[serde(default)]
    pub accuracy: Option<f64>,
    /// 주소나 장소 이름. 주소로 바꾸지 못했으면 없다
    #[serde(default)]
    pub place: Option<String>,
    pub captured_at: DateTime<Utc>,
}

impl Recording {
    /// sori-sync의 SHARED 필드가 같은지
    fn same_shared(&self, other: &Recording) -> bool {
        self.number == other.number
            && self.title == other.title
            && self.created_at == other.created_at
            && self.source_name == other.source_name
            && self.duration == other.duration
            && self.speaker_names == other.speaker_names
            && self.num_speakers == other.num_speakers
            && self.languages == other.languages
            && self.diarize == other.diarize
            && self.tags == other.tags
            && self.tags_edited_at == other.tags_edited_at
            && self.auto_tagged_at == other.auto_tagged_at
            && self.location == other.location
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// 녹음 시작부터 초
    pub time: f32,
    /// 비어 있으면 북마크
    pub text: String,
    /// 동기화할 때 붙는다. 여러 기기의 메모를 합치고 지운 메모를 가린다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 메모를 남긴 기기 id. 없으면 이 기기
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub models: ModelPaths,
    /// 녹음 처리 화면의 기본값
    #[serde(default)]
    pub default_languages: Vec<String>,
    /// None이면 시스템 기본 마이크
    #[serde(default)]
    pub input_device_id: Option<String>,
    /// 마이크와 함께 시스템 소리(통화 상대 목소리 등)도 녹음한다
    #[serde(default)]
    pub capture_system_audio: bool,
    /// 마지막으로 고른 요약 CLI
    #[serde(default)]
    pub summary_provider: Option<Provider>,
    /// None이면 기본 요약 지시문을 쓴다
    #[serde(default)]
    pub summary_prompt: Option<String>,
    /// 앱이 앞에 없어도 동작하는 단축키
    #[serde(default)]
    pub shortcuts: Shortcuts,
    /// 녹음을 멈추거나 파일을 가져오면 바로 받아 적는다
    #[serde(default = "yes")]
    pub auto_process: bool,
    /// 받아 적기가 끝나면 기본 템플릿으로 요약한다. 구독 사용량을 쓰므로 기본은 끈다
    #[serde(default)]
    pub auto_summarize: bool,
    /// 화자 이름을 붙이면 목소리를 기억하고, 다음 녹음에서 같은 사람에게 이름을 붙인다
    #[serde(default = "yes")]
    pub remember_voices: bool,
    /// 자동 처리와 새 녹음의 기본 방식. false면 화자를 나누지 않고 받아 적기만 한다
    #[serde(default = "yes")]
    pub default_diarize: bool,
    /// 회의 앱이 마이크를 쓰기 시작하면 녹음할지 묻는다
    #[serde(default = "yes")]
    pub meeting_detection: bool,
    /// 비어 있으면 기본 템플릿을 쓴다. Store::get_settings가 채운다
    #[serde(default)]
    pub summary_templates: Vec<SummaryTemplate>,
    /// 자동 요약과 요약 화면의 기본 템플릿 id
    #[serde(default)]
    pub default_template: Option<String>,
    /// 기기 간 동기화. 기본은 이 기기에만 저장
    #[serde(default)]
    pub sync: crate::sync::SyncSettings,
}

fn yes() -> bool {
    true
}

impl Settings {
    pub fn defaults(models: ModelPaths) -> Self {
        Self {
            models,
            default_languages: Vec::new(),
            input_device_id: None,
            capture_system_audio: false,
            summary_provider: None,
            summary_prompt: None,
            shortcuts: Shortcuts::default(),
            auto_process: true,
            auto_summarize: false,
            remember_voices: true,
            default_diarize: true,
            meeting_detection: true,
            summary_templates: Vec::new(),
            default_template: None,
            sync: Default::default(),
        }
    }
}

/// 녹음마다 폴더 하나를 쓴다. `meta.json`, 오디오 사본, `transcript.json`
pub struct Store {
    /// 새 녹음 번호를 겹치지 않게 매긴다
    create_lock: std::sync::Mutex<()>,
    /// meta.json을 읽고 고쳐 쓰는 동안 다른 명령이 끼어들어 바꾼 값을 덮지 않게 한다
    meta_lock: std::sync::Mutex<()>,
    root: PathBuf,
    settings_path: PathBuf,
    /// scripts/install-models가 모델을 받는 폴더
    pub models_dir: PathBuf,
}

impl Store {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let root = data_dir.join("recordings");
        fs::create_dir_all(&root)?;
        Ok(Self {
            create_lock: std::sync::Mutex::new(()),
            meta_lock: std::sync::Mutex::new(()),
            root,
            settings_path: data_dir.join("settings.json"),
            models_dir: data_dir.join("models"),
        })
    }

    pub fn get_settings(&self, home: &Path) -> Settings {
        let mut settings: Settings = read_json(&self.settings_path)
            .unwrap_or_else(|_| Settings::defaults(crate::models::default_paths(&self.models_dir, home)));
        if settings.summary_templates.is_empty() {
            // 템플릿이 생기기 전(0.3.0 이하)에 고친 요약 지시문은 회의록 템플릿으로 옮긴다
            settings.summary_templates = crate::summarize::default_templates(settings.summary_prompt.as_deref());
        }
        if !settings.summary_templates.iter().any(|t| Some(&t.id) == settings.default_template.as_ref()) {
            settings.default_template = settings.summary_templates.first().map(|t| t.id.clone());
        }
        settings
    }

    /// 데이터 폴더(설정 파일 옆)의 파일 경로
    pub fn data_file(&self, name: &str) -> PathBuf {
        self.settings_path.with_file_name(name)
    }

    /// 목소리 기억 파일. 녹음 폴더 밖에 두어 녹음을 지워도 남는다
    pub fn voices_path(&self) -> PathBuf {
        self.settings_path.with_file_name("voices.json")
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_json(&self.settings_path, settings)
    }

    /// 지금까지 쓴 가장 큰 번호 + 1. 지운 녹음의 번호를 다시 쓰지 않도록 기록 파일에도 남긴다
    fn next_number(&self) -> Result<u32> {
        let counter = self.settings_path.with_file_name("last-number");
        let saved: u32 = fs::read_to_string(&counter).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let listed = self.list()?.iter().map(|r| r.number).max().unwrap_or(0);
        let next = saved.max(listed) + 1;
        fs::write(&counter, next.to_string())?;
        Ok(next)
    }

    /// 번호가 없는 녹음(0.4.3 이하)에 만든 순서대로 번호를 붙인다
    pub fn assign_missing_numbers(&self) -> Result<()> {
        let _guard = self.create_lock.lock().unwrap();
        let mut all = self.list()?;
        if all.iter().all(|r| r.number > 0) {
            return Ok(());
        }
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let mut next = all.iter().map(|r| r.number).max().unwrap_or(0);
        for r in all.iter().filter(|r| r.number == 0) {
            next += 1;
            let n = next;
            self.update(&r.id, |m| m.number = n)?;
        }
        fs::write(self.settings_path.with_file_name("last-number"), next.to_string())?;
        Ok(())
    }

    /// "#12", "12", 또는 id로 녹음을 찾는다
    pub fn resolve(&self, key: &str) -> Result<Recording> {
        let key = key.trim();
        if let Ok(n) = key.trim_start_matches('#').parse::<u32>() {
            return self.list()?.into_iter().find(|r| r.number == n).ok_or_else(|| eyre!("#{n} 녹음이 없습니다"));
        }
        self.get(key)
    }

    pub fn list(&self) -> Result<Vec<Recording>> {
        let mut recordings: Vec<Recording> = fs::read_dir(&self.root)?
            .filter_map(|e| e.ok())
            .filter_map(|e| read_json(&e.path().join("meta.json")).ok())
            .collect();
        recordings.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(recordings)
    }

    pub fn get(&self, id: &str) -> Result<Recording> {
        read_json(&self.dir(id)?.join("meta.json")).wrap_err_with(|| format!("녹음을 찾을 수 없습니다: {id}"))
    }

    pub fn save(&self, recording: &Recording) -> Result<()> {
        let _guard = self.meta_lock.lock().unwrap();
        let path = self.dir(&recording.id)?.join("meta.json");
        let mut recording = recording.clone();
        match read_json::<Recording>(&path) {
            Ok(old) if old.same_shared(&recording) => recording.updated_at = old.updated_at,
            _ => recording.updated_at = Some(Utc::now()),
        }
        write_json(&path, &recording)
    }

    /// 잠근 채 최신 meta.json을 다시 읽어 고치고 저장한다. 오래 걸리는 작업 뒤에 저장할 때는
    /// 처음에 읽어 둔 값을 쓰지 말고 이것으로 필요한 필드만 바꾼다
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Recording)) -> Result<Recording> {
        let _guard = self.meta_lock.lock().unwrap();
        let path = self.dir(id)?.join("meta.json");
        let mut recording: Recording = read_json(&path).wrap_err_with(|| format!("녹음을 찾을 수 없습니다: {id}"))?;
        let before = recording.clone();
        change(&mut recording);
        if !before.same_shared(&recording) {
            recording.updated_at = Some(Utc::now());
        }
        write_json(&path, &recording)?;
        Ok(recording)
    }

    /// 동기화 엔진용: meta.json을 JSON 값 그대로 고친다. 고친 시각은 엔진이 정하므로 건드리지 않는다.
    /// 저장하기 전에 Recording으로 읽히는지 확인해 앱이 못 읽는 파일을 만들지 않는다
    pub fn update_raw(&self, id: &str, change: &mut dyn FnMut(&mut serde_json::Value)) -> Result<serde_json::Value> {
        let _guard = self.meta_lock.lock().unwrap();
        let path = self.dir(id)?.join("meta.json");
        let mut value: serde_json::Value = read_json(&path).wrap_err_with(|| format!("녹음을 찾을 수 없습니다: {id}"))?;
        change(&mut value);
        serde_json::from_value::<Recording>(value.clone()).wrap_err("동기화한 meta.json을 읽을 수 없습니다")?;
        write_json(&path, &value)?;
        Ok(value)
    }

    pub fn read_raw(&self, id: &str) -> Result<serde_json::Value> {
        read_json(&self.dir(id)?.join("meta.json"))
    }

    /// 동기화 엔진용: 다른 기기에서 온 녹음을 그 id 그대로 만든다
    pub fn create_raw(&self, id: &str, meta: serde_json::Value) -> Result<()> {
        let _guard = self.create_lock.lock().unwrap();
        serde_json::from_value::<Recording>(meta.clone()).wrap_err("동기화한 meta.json을 읽을 수 없습니다")?;
        let dir = self.dir(id)?;
        fs::create_dir_all(&dir)?;
        write_json(&dir.join("meta.json"), &meta)
    }

    pub fn recording_dir(&self, id: &str) -> Result<PathBuf> {
        self.dir(id)
    }

    pub fn ids(&self) -> Result<Vec<String>> {
        Ok(fs::read_dir(&self.root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("meta.json").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect())
    }

    pub fn import(&self, source: &Path) -> Result<Recording> {
        let source_name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| eyre!("파일 이름이 없습니다: {}", source.display()))?;
        let ext = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let audio_file = if ext.is_empty() { "audio".to_string() } else { format!("audio.{ext}") };
        let title = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| source_name.clone());

        let recording = self.create(title, source_name, audio_file, Status::New)?;
        let dest = self.audio_path(&recording)?;
        if let Err(e) = fs::copy(source, &dest) {
            let _ = self.delete(&recording.id);
            return Err(e).wrap_err_with(|| format!("파일을 복사하지 못했습니다: {}", source.display()));
        }
        Ok(recording)
    }

    /// 앱에서 녹음할 새 항목을 만든다. 트랙 파일은 녹음기가 채우고, 멈출 때 audio.wav로 섞는다
    pub fn create_for_recording(&self, title: String, source_name: String, tracks: Vec<String>) -> Result<Recording> {
        let mut recording = self.create(title, source_name, "audio.wav".to_string(), Status::Recording)?;
        recording.tracks = tracks;
        self.save(&recording)?;
        Ok(recording)
    }

    pub fn track_paths(&self, recording: &Recording) -> Result<Vec<PathBuf>> {
        let dir = self.dir(&recording.id)?;
        Ok(recording.tracks.iter().map(|t| dir.join(t)).collect())
    }

    fn create(&self, title: String, source_name: String, audio_file: String, status: Status) -> Result<Recording> {
        let _guard = self.create_lock.lock().unwrap();
        let number = self.next_number()?;
        let id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(self.root.join(&id))?;
        let recording = Recording {
            id,
            number,
            title,
            created_at: Utc::now(),
            source_name,
            audio_file,
            status,
            error: None,
            duration: None,
            speaker_names: BTreeMap::new(),
            num_speakers: None,
            languages: Vec::new(),
            tracks: Vec::new(),
            notes: Vec::new(),
            voice_ids: BTreeMap::new(),
            diarize: true,
            updated_at: None,
            tags: Vec::new(),
            tags_edited_at: None,
            auto_tagged_at: None,
            location: None,
        };
        self.save(&recording)?;
        Ok(recording)
    }

    pub fn audio_path(&self, recording: &Recording) -> Result<PathBuf> {
        Ok(self.dir(&recording.id)?.join(&recording.audio_file))
    }

    pub fn get_transcript(&self, id: &str) -> Result<Option<Transcript>> {
        let path = self.dir(id)?.join("transcript.json");
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// 전사 파일의 수정 시각. 없으면 None
    pub fn transcript_modified(&self, id: &str) -> Option<std::time::SystemTime> {
        fs::metadata(self.dir(id).ok()?.join("transcript.json")).ok()?.modified().ok()
    }

    pub fn save_transcript(&self, id: &str, transcript: &Transcript) -> Result<()> {
        write_json(&self.dir(id)?.join("transcript.json"), transcript)
    }

    /// 가장 최근 요약. summary.json에 따로 두어 Raycast, MCP 서버처럼 파일을 직접 읽는 쪽이 하나만 보면 되게 한다
    pub fn get_summary(&self, id: &str) -> Result<Option<Summary>> {
        let path = self.dir(id)?.join("summary.json");
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// 이 녹음의 모든 요약, 오래된 것부터. summaries.json이 없으면(0.3.0 이하) summary.json 하나를 목록으로 본다
    pub fn get_summaries(&self, id: &str) -> Result<Vec<Summary>> {
        let path = self.dir(id)?.join("summaries.json");
        if path.exists() {
            return read_json(&path);
        }
        Ok(self.get_summary(id)?.into_iter().collect())
    }

    pub fn save_summary(&self, id: &str, summary: &Summary) -> Result<()> {
        let mut all = self.get_summaries(id)?;
        all.push(summary.clone());
        write_json(&self.dir(id)?.join("summaries.json"), &all)?;
        write_json(&self.dir(id)?.join("summary.json"), summary)
    }

    pub fn delete_summary(&self, id: &str, summary_id: &str) -> Result<()> {
        let mut all = self.get_summaries(id)?;
        all.retain(|s| s.id != summary_id);
        let dir = self.dir(id)?;
        write_json(&dir.join("summaries.json"), &all)?;
        match all.last() {
            Some(latest) => write_json(&dir.join("summary.json"), latest),
            None => {
                let _ = fs::remove_file(dir.join("summary.json"));
                Ok(())
            }
        }
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        fs::remove_dir_all(self.dir(id)?)?;
        Ok(())
    }

    /// 프론트에서 온 id로 녹음 폴더 밖의 경로를 만들지 못하게 막는다
    fn dir(&self, id: &str) -> Result<PathBuf> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(eyre!("잘못된 녹음 id입니다: {id}"));
        }
        Ok(self.root.join(id))
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// 임시 파일에 쓰고 이름을 바꿔서, 쓰는 도중 앱이 꺼져도 기존 파일이 깨지지 않게 한다
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    // 두 곳에서 같은 파일을 동시에 저장해도 임시 파일이 겹치지 않게 이름을 매번 다르게 한다
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod number_tests {
    use super::*;

    fn store() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sori-store-{}", uuid::Uuid::new_v4()));
        (Store::new(dir.clone()).unwrap(), dir)
    }

    #[test]
    fn 번호는_순서대로_붙고_지운_번호는_다시_쓰지_않는다() {
        let (store, dir) = store();

        let a = store.create_for_recording("a".into(), "s".into(), vec![]).unwrap();
        let b = store.create_for_recording("b".into(), "s".into(), vec![]).unwrap();
        store.delete(&b.id).unwrap();
        let c = store.create_for_recording("c".into(), "s".into(), vec![]).unwrap();

        assert_eq!((a.number, b.number, c.number), (1, 2, 3));
        assert_eq!(store.resolve("#3").unwrap().id, c.id);
        assert_eq!(store.resolve("1").unwrap().id, a.id);
        assert!(store.resolve("#2").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 번호가_없는_예전_녹음은_만든_순서대로_채운다() {
        let (store, dir) = store();
        let old = store.create_for_recording("old".into(), "s".into(), vec![]).unwrap();
        let new = store.create_for_recording("new".into(), "s".into(), vec![]).unwrap();
        for id in [&old.id, &new.id] {
            store.update(id, |r| r.number = 0).unwrap();
        }
        let _ = fs::remove_file(dir.join("last-number"));

        store.assign_missing_numbers().unwrap();

        assert_eq!(store.get(&old.id).unwrap().number, 1);
        assert_eq!(store.get(&new.id).unwrap().number, 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
