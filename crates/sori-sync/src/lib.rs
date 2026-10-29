//! sori 기기 간 동기화 엔진. 클라우드(폴더, OneDrive, Google Drive)와 기기 저장소 사이에서 녹음을 맞춘다.
//! 앱 종류(데스크톱, 모바일)와 상관없이 쓰도록 녹음 형식은 JSON 값으로만 다룬다.
//! @see docs/sync.md

mod engine;
mod folder;
mod local;
mod meta;
mod remote;
mod state;

pub use engine::{download_audio, remote_audio_name, sync, SyncReport};
pub use folder::FolderRemote;
pub use local::{AudioUpload, Local};
pub use remote::{Progress, Remote, RemoteFile};
