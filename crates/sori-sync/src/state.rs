//! 지난번 동기화 때의 지문. 다음 동기화에서 어느 쪽이 바뀌었는지 가린다

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eyre::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// 어느 클라우드와 맞춘 상태인지. 바뀌면(다른 클라우드로 옮김) 처음부터 다시 맞춘다
    pub remote: String,
    pub recordings: BTreeMap<String, RecordingState>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
    /// 파일 이름 → 지난번 지문
    pub files: BTreeMap<String, FileState>,
    /// 지난번 동기화 직후 화면에 있던 다른 기기 메모 id. 사라졌으면 이 기기에서 지운 것이다
    pub others_notes: BTreeSet<String>,
    /// 이 기기가 지운 메모 id. 내 메모 파일에 함께 올린다
    pub deleted_notes: BTreeSet<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileState {
    pub local: Option<u64>,
    pub remote: Option<String>,
}

impl State {
    pub fn load(path: &Path, remote: &str) -> State {
        let state: State = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if state.remote == remote { state } else { State { remote: remote.to_string(), ..Default::default() } }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
