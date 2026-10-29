use std::path::Path;

use chrono::{DateTime, Utc};
use eyre::Result;

/// 클라우드의 파일 하나. path는 sori 루트 기준이고 구분자는 `/`다
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub path: String,
    /// 내용이 바뀌면 달라지는 값(etag, 수정 시각과 크기 등). 지난번 동기화 이후 바뀌었는지 비교만 한다
    pub version: String,
    pub size: u64,
    pub modified: DateTime<Utc>,
}

/// 받은 바이트, 전체 바이트(모르면 0)
pub type Progress<'a> = &'a (dyn Fn(u64, u64) + Sync);

/// 클라우드 공통 동작. 폴더 방식과 API 방식이 같은 모양으로 구현한다
pub trait Remote: Send + Sync {
    /// dir 아래 모든 파일(하위 폴더 포함). dir이 없으면 빈 목록
    fn list(&self, dir: &str) -> Result<Vec<RemoteFile>>;
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    /// 쓰고 나서의 버전을 돌려준다. 상위 폴더가 없으면 만든다
    fn write(&self, path: &str, data: &[u8]) -> Result<String>;
    fn upload(&self, path: &str, source: &Path, progress: Progress) -> Result<String>;
    fn download(&self, path: &str, dest: &Path, progress: Progress) -> Result<()>;
    /// 없으면 아무것도 하지 않는다
    fn delete_file(&self, path: &str) -> Result<()>;
    /// 하위 파일까지 지운다. 없으면 아무것도 하지 않는다
    fn delete_dir(&self, dir: &str) -> Result<()>;
}
