//! 폴더 방식 클라우드. iCloud Drive처럼 운영체제가 폴더를 동기화해 주는 경우에 쓴다.
//! 파일 내용을 아직 받지 않은 경우(macOS의 dataless 파일, iCloud for Windows의 자리표시자)에도 목록과 크기는 보이고,
//! 읽으면 운영체제가 받아 온다.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};

use crate::remote::{Progress, Remote, RemoteFile};

/// 파일을 읽기 전에 클라우드에서 받아 오게 하는 함수
pub type Materialize = Box<dyn Fn(&Path) -> Result<()> + Send + Sync>;

pub struct FolderRemote {
    root: PathBuf,
    materialize: Option<Materialize>,
}

impl FolderRemote {
    pub fn new(root: PathBuf) -> Self {
        Self { root, materialize: None }
    }

    /// iOS는 받지 않은 iCloud 파일을 그냥 읽으면 실패한다. 읽기 전에 NSFileCoordinator로 받아 오는 함수를 넣는다.
    /// macOS는 읽기만 해도 운영체제가 받아 와서 필요 없다
    pub fn with_materialize(mut self, materialize: Materialize) -> Self {
        self.materialize = Some(materialize);
        self
    }

    fn fetch(&self, path: &Path) -> Result<()> {
        match &self.materialize {
            Some(m) => m(path).wrap_err_with(|| format!("iCloud에서 파일을 받지 못했습니다: {}", path.display())),
            None => Ok(()),
        }
    }

    fn path(&self, rel: &str) -> Result<PathBuf> {
        // 클라우드 목록에서 온 이름으로 루트 밖을 가리키지 못하게 한다
        if rel.split('/').any(|p| p == ".." || p.is_empty()) || rel.starts_with('/') || rel.contains('\\') {
            return Err(eyre!("잘못된 경로입니다: {rel}"));
        }
        Ok(self.root.join(rel))
    }
}

/// 예전 macOS의 iCloud는 받지 않은 파일을 `.이름.icloud`로 둔다. 원래 이름을 돌려준다
fn icloud_placeholder(name: &str) -> Option<&str> {
    name.strip_prefix('.')?.strip_suffix(".icloud")
}

fn version_of(meta: &fs::Metadata) -> (String, DateTime<Utc>) {
    let modified: DateTime<Utc> = meta.modified().map(DateTime::from).unwrap_or_else(|_| Utc::now());
    (format!("{}-{}", modified.timestamp_nanos_opt().unwrap_or_default(), meta.len()), modified)
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<RemoteFile>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).wrap_err_with(|| format!("폴더를 읽지 못했습니다: {}", dir.display())),
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            if !name.starts_with('.') {
                walk(&entry.path(), &format!("{rel}{name}/"), out)?;
            }
            continue;
        }
        // 쓰는 중인 임시 파일과 숨김 파일은 건너뛴다. 받지 않은 iCloud 파일은 원래 이름으로 보인다
        let name = match icloud_placeholder(&name) {
            Some(real) => real.to_string(),
            None if name.starts_with('.') || name.ends_with(".tmp") => continue,
            None => name,
        };
        let (version, modified) = version_of(&meta);
        out.push(RemoteFile { path: format!("{rel}{name}"), version, size: meta.len(), modified });
    }
    Ok(())
}

fn copy_with_progress(from: &Path, to: &Path, progress: Progress) -> Result<()> {
    let mut input = fs::File::open(from).wrap_err_with(|| format!("파일을 열지 못했습니다: {}", from.display()))?;
    let total = input.metadata().map(|m| m.len()).unwrap_or(0);
    let mut output = fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    output.sync_all()?;
    Ok(())
}

/// 같은 폴더의 임시 파일에 쓰고 이름을 바꾼다. 다른 기기가 반쯤 쓴 파일을 읽지 않게 한다
fn tmp_beside(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()))
}

impl Remote for FolderRemote {
    fn list(&self, dir: &str) -> Result<Vec<RemoteFile>> {
        let mut out = Vec::new();
        walk(&self.path(dir)?, &format!("{dir}/"), &mut out)?;
        Ok(out)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let p = self.path(path)?;
        self.fetch(&p)?;
        fs::read(&p).wrap_err_with(|| format!("파일을 읽지 못했습니다: {}", p.display()))
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<String> {
        let dest = self.path(path)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = tmp_beside(&dest);
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &dest)?;
        Ok(version_of(&fs::metadata(&dest)?).0)
    }

    fn upload(&self, path: &str, source: &Path, progress: Progress) -> Result<String> {
        let dest = self.path(path)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = tmp_beside(&dest);
        if let Err(e) = copy_with_progress(source, &tmp, progress) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        fs::rename(&tmp, &dest)?;
        Ok(version_of(&fs::metadata(&dest)?).0)
    }

    fn download(&self, path: &str, dest: &Path, progress: Progress) -> Result<()> {
        let source = self.path(path)?;
        self.fetch(&source)?;
        let tmp = tmp_beside(dest);
        if let Err(e) = copy_with_progress(&source, &tmp, progress) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        fs::rename(&tmp, dest)?;
        Ok(())
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        match fs::remove_file(self.path(path)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    fn delete_dir(&self, dir: &str) -> Result<()> {
        match fs::remove_dir_all(self.path(dir)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}
