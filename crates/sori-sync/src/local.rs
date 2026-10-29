use std::path::{Path, PathBuf};

use eyre::Result;
use serde_json::Value;

/// 기기 저장소. 녹음마다 폴더 하나에 meta.json, 오디오, transcript.json 등을 둔다(데스크톱 store.rs와 같은 구조).
/// meta.json은 앱마다 필드가 달라 JSON 값으로 다룬다. 고칠 때는 앱의 잠금을 지키도록 update_meta를 거친다
pub trait Local {
    /// 이 기기의 고정 id. 메모 파일 이름에 쓴다
    fn device_id(&self) -> String;
    fn recording_ids(&self) -> Result<Vec<String>>;
    fn recording_dir(&self, id: &str) -> PathBuf;
    /// 녹음 중이거나 처리 중이면 true. 이런 녹음은 이번 동기화에서 건너뛴다
    fn is_busy(&self, id: &str) -> bool;
    fn read_meta(&self, id: &str) -> Result<Value>;
    fn update_meta(&self, id: &str, change: &mut dyn FnMut(&mut Value)) -> Result<Value>;
    /// 다른 기기에서 온 녹음의 폴더와 meta.json을 만든다
    fn create_recording(&self, id: &str, meta: Value) -> Result<()>;
    fn delete_recording(&self, id: &str) -> Result<()>;
    /// 클라우드에 올릴 오디오를 준비한다. 압축 형식이면 원본 그대로, WAV면 압축한 임시 파일
    fn prepare_audio(&self, id: &str, source: &Path) -> Result<AudioUpload>;
    /// 다른 기기에서 전사가 왔을 때. 상태를 "완료"로 바꾸는 등
    fn transcript_arrived(&self, id: &str) -> Result<()>;
}

pub struct AudioUpload {
    pub path: PathBuf,
    /// 클라우드 파일 확장자(m4a, flac 등)
    pub ext: String,
    /// 올린 뒤 지울 임시 파일인지
    pub temporary: bool,
}
