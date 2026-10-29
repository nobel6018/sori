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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// 녹음 시작부터 초
    pub time: f32,
    /// 비어 있으면 북마크
    pub text: String,
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
        }
    }
}

/// 녹음마다 폴더 하나를 쓴다. `meta.json`, 오디오 사본, `transcript.json`
pub struct Store {
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

    /// 목소리 기억 파일. 녹음 폴더 밖에 두어 녹음을 지워도 남는다
    pub fn voices_path(&self) -> PathBuf {
        self.settings_path.with_file_name("voices.json")
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        write_json(&self.settings_path, settings)
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
        write_json(&self.dir(&recording.id)?.join("meta.json"), recording)
    }

    /// 잠근 채 최신 meta.json을 다시 읽어 고치고 저장한다. 오래 걸리는 작업 뒤에 저장할 때는
    /// 처음에 읽어 둔 값을 쓰지 말고 이것으로 필요한 필드만 바꾼다
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Recording)) -> Result<Recording> {
        let _guard = self.meta_lock.lock().unwrap();
        let path = self.dir(id)?.join("meta.json");
        let mut recording: Recording = read_json(&path).wrap_err_with(|| format!("녹음을 찾을 수 없습니다: {id}"))?;
        change(&mut recording);
        write_json(&path, &recording)?;
        Ok(recording)
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
        let id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(self.root.join(&id))?;
        let recording = Recording {
            id,
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
