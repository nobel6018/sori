use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::engine::{ModelPaths, Transcript};

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
}

impl Settings {
    pub fn defaults(home: &Path) -> Self {
        Self {
            models: ModelPaths {
                whisper: home.join("whisper-models").join("ggml-large-v3-turbo.bin"),
                segmentation: home
                    .join("sherpa-onnx")
                    .join("models")
                    .join("sherpa-onnx-pyannote-segmentation-3-0")
                    .join("model.onnx"),
                embedding: home
                    .join("sherpa-onnx")
                    .join("models")
                    .join("3dspeaker_eres2net_base_zh-cn_16k.onnx"),
            },
            default_languages: Vec::new(),
            input_device_id: None,
            capture_system_audio: false,
        }
    }
}

/// 녹음마다 폴더 하나를 쓴다. `meta.json`, 오디오 사본, `transcript.json`
pub struct Store {
    root: PathBuf,
    settings_path: PathBuf,
}

impl Store {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let root = data_dir.join("recordings");
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            settings_path: data_dir.join("settings.json"),
        })
    }

    pub fn get_settings(&self, home: &Path) -> Settings {
        read_json(&self.settings_path).unwrap_or_else(|_| Settings::defaults(home))
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
        write_json(&self.dir(&recording.id)?.join("meta.json"), recording)
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

    pub fn save_transcript(&self, id: &str, transcript: &Transcript) -> Result<()> {
        write_json(&self.dir(id)?.join("transcript.json"), transcript)
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
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
