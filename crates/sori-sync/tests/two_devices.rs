//! 기기 두 대(A, B)가 폴더 클라우드 하나를 함께 쓰는 상황을 흉내 낸다

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sori_sync::{download_audio, sync, AudioUpload, FolderRemote, Local};
use tempfile::TempDir;

struct Device {
    name: String,
    root: PathBuf,
    state: PathBuf,
    _tmp: TempDir,
}

impl Device {
    fn new(name: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("recordings");
        fs::create_dir_all(&root).unwrap();
        Device { name: name.into(), state: tmp.path().join("sync-state.json"), root, _tmp: tmp }
    }

    fn record(&self, number: u64, title: &str, created: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(self.root.join(&id)).unwrap();
        fs::write(self.root.join(&id).join("audio.m4a"), b"fake-audio").unwrap();
        let meta = json!({ "id": id, "number": number, "title": title, "createdAt": created, "sourceName": "", "audioFile": "audio.m4a", "status": "new", "notes": [] });
        self.create_recording(&id, meta).unwrap();
        id
    }

    fn meta(&self, id: &str) -> Value {
        self.read_meta(id).unwrap()
    }

    fn sync(&self, remote: &FolderRemote) -> sori_sync::SyncReport {
        let report = sync(self, remote, "test", &self.state, &|_, _| {}).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        report
    }
}

impl Local for Device {
    fn device_id(&self) -> String {
        self.name.clone()
    }
    fn recording_ids(&self) -> eyre::Result<Vec<String>> {
        Ok(fs::read_dir(&self.root)?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    }
    fn recording_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }
    fn is_busy(&self, _: &str) -> bool {
        false
    }
    fn read_meta(&self, id: &str) -> eyre::Result<Value> {
        Ok(serde_json::from_slice(&fs::read(self.root.join(id).join("meta.json"))?)?)
    }
    fn update_meta(&self, id: &str, change: &mut dyn FnMut(&mut Value)) -> eyre::Result<Value> {
        let mut m = self.read_meta(id)?;
        change(&mut m);
        fs::write(self.root.join(id).join("meta.json"), serde_json::to_vec(&m)?)?;
        Ok(m)
    }
    fn create_recording(&self, id: &str, meta: Value) -> eyre::Result<()> {
        fs::create_dir_all(self.root.join(id))?;
        fs::write(self.root.join(id).join("meta.json"), serde_json::to_vec(&meta)?)?;
        Ok(())
    }
    fn delete_recording(&self, id: &str) -> eyre::Result<()> {
        fs::remove_dir_all(self.root.join(id))?;
        Ok(())
    }
    fn prepare_audio(&self, _: &str, source: &Path) -> eyre::Result<AudioUpload> {
        Ok(AudioUpload { path: source.to_path_buf(), ext: "m4a".into(), temporary: false })
    }
    fn transcript_arrived(&self, id: &str) -> eyre::Result<()> {
        self.update_meta(id, &mut |m| m["status"] = json!("done")).map(|_| ())
    }
}

fn cloud() -> (TempDir, FolderRemote) {
    let tmp = TempDir::new().unwrap();
    let remote = FolderRemote::new(tmp.path().to_path_buf());
    (tmp, remote)
}

fn tick() {
    // 파일 수정 시각으로 순서를 가리는 경우가 있어 조금 기다린다
    std::thread::sleep(std::time::Duration::from_millis(20));
}

#[test]
fn 한_기기에서_만든_녹음이_다른_기기에_오고_오디오는_받기_전까지_없다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "회의", "2026-10-29T01:00:00Z");

    a.sync(&remote);
    b.sync(&remote);

    let meta = b.meta(&id);
    assert_eq!(meta["title"], "회의");
    assert_eq!(meta["audioFile"], "audio.m4a");
    assert!(!b.recording_dir(&id).join("audio.m4a").exists());

    download_audio(&b, &remote, &id, &|_, _| {}).unwrap();
    assert_eq!(fs::read(b.recording_dir(&id).join("audio.m4a")).unwrap(), b"fake-audio");
}

#[test]
fn 전사는_처리한_기기에서_다른_기기로_가고_상태가_완료가_된다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "회의", "2026-10-29T01:00:00Z");
    a.sync(&remote);
    b.sync(&remote);

    fs::write(b.recording_dir(&id).join("transcript.json"), br#"{"utterances":[]}"#).unwrap();
    b.sync(&remote);
    a.sync(&remote);

    assert!(a.recording_dir(&id).join("transcript.json").exists());
    assert_eq!(a.meta(&id)["status"], "done");
}

#[test]
fn 제목은_마지막에_고친_기기를_따른다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "처음", "2026-10-29T01:00:00Z");
    a.sync(&remote);
    b.sync(&remote);

    a.update_meta(&id, &mut |m| m["title"] = json!("A가 고침")).unwrap();
    tick();
    b.update_meta(&id, &mut |m| m["title"] = json!("B가 나중에 고침")).unwrap();
    a.sync(&remote);
    tick();
    b.sync(&remote);
    a.sync(&remote);

    assert_eq!(a.meta(&id)["title"], "B가 나중에 고침");
    assert_eq!(b.meta(&id)["title"], "B가 나중에 고침");
}

#[test]
fn 지운_녹음은_다른_기기에서도_지워진다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "회의", "2026-10-29T01:00:00Z");
    a.sync(&remote);
    b.sync(&remote);

    a.delete_recording(&id).unwrap();
    a.sync(&remote);
    let report = b.sync(&remote);

    assert_eq!(report.deleted_local, 1);
    assert!(!b.recording_dir(&id).exists());
    // 다시 동기화해도 되살아나지 않는다
    a.sync(&remote);
    assert!(!a.recording_dir(&id).exists());
}

#[test]
fn 메모는_두_기기_것을_합치고_지운_메모는_양쪽에서_사라진다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "회의", "2026-10-29T01:00:00Z");
    // 동기화 전 예전 메모: id와 기기가 없다
    a.update_meta(&id, &mut |m| m["notes"] = json!([{ "time": 5.0, "text": "A 메모" }])).unwrap();
    a.sync(&remote);
    b.sync(&remote);

    b.update_meta(&id, &mut |m| {
        m["notes"].as_array_mut().unwrap().push(json!({ "time": 1.0, "text": "B 메모", "id": "b-1", "device": "B" }));
    })
    .unwrap();
    b.sync(&remote);
    a.sync(&remote);

    let texts = |d: &Device| d.meta(&id)["notes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(texts(&a), vec!["B 메모", "A 메모"]);

    // A가 B의 메모를 지운다
    a.update_meta(&id, &mut |m| m["notes"].as_array_mut().unwrap().retain(|n| n["device"] != "B")).unwrap();
    a.sync(&remote);
    b.sync(&remote);
    a.sync(&remote);

    assert_eq!(texts(&a), vec!["A 메모"]);
    assert_eq!(texts(&b), vec!["A 메모"]);
}

#[test]
fn 오프라인에서_겹친_번호는_늦게_만든_녹음이_새_번호를_받는다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let early = a.record(5, "먼저", "2026-10-29T01:00:00Z");
    let late = b.record(5, "나중", "2026-10-29T02:00:00Z");

    a.sync(&remote);
    b.sync(&remote);
    a.sync(&remote);

    for d in [&a, &b] {
        assert_eq!(d.meta(&early)["number"], 5);
        assert_eq!(d.meta(&late)["number"], 6);
    }
}

#[test]
fn 바뀐_것이_없으면_아무것도_옮기지_않는다() {
    let (_c, remote) = cloud();
    let a = Device::new("A");
    a.record(1, "회의", "2026-10-29T01:00:00Z");
    a.sync(&remote);

    let report = a.sync(&remote);

    assert_eq!((report.uploaded, report.downloaded), (0, 0));
}

#[test]
fn 태그와_위치가_다른_기기로_가고_나중에_고친_태그가_이긴다() {
    let (_c, remote) = cloud();
    let (a, b) = (Device::new("A"), Device::new("B"));
    let id = a.record(1, "회의", "2026-10-29T01:00:00Z");
    let place = json!({ "lat": 37.5665, "lon": 126.978, "accuracy": 35.0, "place": "서울 중구", "capturedAt": "2026-10-29T01:00:01Z" });
    a.update_meta(&id, &mut |m| {
        m["tags"] = json!(["업무/영업"]);
        m["location"] = place.clone();
        m["updatedAt"] = json!("2026-10-29T01:00:02Z");
    })
    .unwrap();
    a.sync(&remote);
    b.sync(&remote);
    assert_eq!(b.meta(&id)["tags"], json!(["업무/영업"]));
    assert_eq!(b.meta(&id)["location"], place);

    tick();
    b.update_meta(&id, &mut |m| {
        m["tags"] = json!(["업무/영업", "고객미팅"]);
        m["tagsEditedAt"] = json!("2026-10-29T02:00:00Z");
        m["updatedAt"] = json!("2026-10-29T02:00:00Z");
    })
    .unwrap();
    b.sync(&remote);
    a.sync(&remote);
    assert_eq!(a.meta(&id)["tags"], json!(["업무/영업", "고객미팅"]));
    assert_eq!(a.meta(&id)["tagsEditedAt"], "2026-10-29T02:00:00Z");
}
