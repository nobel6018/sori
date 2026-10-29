//! 동기화 순서와 충돌 규칙. @see docs/sync.md

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};
use serde_json::{json, Value};

use crate::local::Local;
use crate::meta::{self, fingerprint, fnv, NotesFile};
use crate::remote::{Progress, Remote, RemoteFile};
use crate::state::{FileState, RecordingState, State};

/// 처리한 데스크톱이 쓰고 다른 기기는 읽기만 하는 파일
const RESULT_FILES: &[&str] = &["transcript.json", "summaries.json", "summary.json"];

#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    /// 녹음 하나가 실패해도 나머지는 계속 맞춘다. 실패한 녹음과 이유
    pub errors: Vec<String>,
}

/// 기기 저장소와 클라우드를 한 번 맞춘다.
/// remote_key는 클라우드를 구분하는 값(폴더 경로, 계정 id 등)이다. 바뀌면 지난번 상태를 버리고 처음부터 맞춘다
pub fn sync(local: &dyn Local, remote: &dyn Remote, remote_key: &str, state_path: &Path, progress: Progress) -> Result<SyncReport> {
    let mut state = State::load(state_path, remote_key);
    let mut report = SyncReport::default();

    let mut remote_recordings: BTreeMap<String, BTreeMap<String, RemoteFile>> = BTreeMap::new();
    for file in remote.list("recordings")? {
        let Some((id, name)) = file.path.strip_prefix("recordings/").and_then(|p| p.split_once('/')) else { continue };
        if valid_id(id) {
            remote_recordings.entry(id.to_string()).or_default().insert(name.to_string(), file);
        }
    }
    let tombstones: BTreeSet<String> = remote
        .list("deleted")?
        .iter()
        .filter_map(|f| f.path.strip_prefix("deleted/")?.strip_suffix(".json"))
        .filter(|id| valid_id(id))
        .map(str::to_string)
        .collect();
    let mut local_ids: BTreeSet<String> = local.recording_ids()?.into_iter().filter(|id| valid_id(id)).collect();

    // 1. 다른 기기에서 지운 녹음
    for id in &tombstones {
        if local_ids.contains(id) {
            if local.is_busy(id) {
                continue;
            }
            local.delete_recording(id)?;
            local_ids.remove(id);
            report.deleted_local += 1;
        }
        state.recordings.remove(id);
        if remote_recordings.remove(id).is_some() {
            remote.delete_dir(&format!("recordings/{id}"))?;
        }
    }

    // 2. 이 기기에서 지운 녹음: 지난번에 맞춘 적이 있는데 지금 없다
    let gone: Vec<String> = state.recordings.keys().filter(|id| !local_ids.contains(*id)).cloned().collect();
    for id in gone {
        let body = json!({ "deletedAt": Utc::now().to_rfc3339(), "device": local.device_id() });
        remote.write(&format!("deleted/{id}.json"), serde_json::to_vec_pretty(&body)?.as_slice())?;
        remote.delete_dir(&format!("recordings/{id}"))?;
        remote_recordings.remove(&id);
        state.recordings.remove(&id);
        report.deleted_remote += 1;
    }

    // 3. 다른 기기에서 만든 녹음
    for (id, files) in &remote_recordings {
        if local_ids.contains(id) || tombstones.contains(id) {
            continue;
        }
        // meta.json이 아직 안 올라왔으면(올리는 중) 다음 동기화에서 가져온다
        let Some(meta_file) = files.get("meta.json") else { continue };
        let result = (|| -> Result<()> {
            let remote_meta: Value = serde_json::from_slice(&remote.read(&meta_file.path)?)?;
            let local_meta = meta::new_local(&remote_meta, files.contains_key("transcript.json"));
            local.create_recording(id, local_meta.clone())?;
            let mut rs = RecordingState::default();
            rs.files.insert(
                "meta.json".into(),
                FileState { local: Some(fingerprint(&meta::project(&local_meta))), remote: Some(meta_file.version.clone()) },
            );
            state.recordings.insert(id.clone(), rs);
            Ok(())
        })();
        match result {
            Ok(()) => {
                local_ids.insert(id.clone());
                report.downloaded += 1;
            }
            Err(e) => report.errors.push(format!("{id}: {e:#}")),
        }
    }

    // 4. 기기마다 따로 매긴 번호가 겹치면 고친다
    if let Err(e) = renumber(local, &local_ids) {
        report.errors.push(format!("번호 맞추기: {e:#}"));
    }

    // 5. 녹음마다 파일 맞추기
    let empty = BTreeMap::new();
    for id in &local_ids {
        if local.is_busy(id) {
            continue;
        }
        let files = remote_recordings.get(id).unwrap_or(&empty);
        let mut rs = state.recordings.get(id).cloned().unwrap_or_default();
        let result = sync_recording(local, remote, id, files, &mut rs, &mut report, progress);
        // 실패해도 성공한 파일까지의 지문은 남겨, 다음에 같은 파일을 다시 옮기지 않게 한다
        state.recordings.insert(id.clone(), rs);
        if let Err(e) = result {
            report.errors.push(format!("{id}: {e:#}"));
        }
    }

    state.save(state_path)?;
    Ok(report)
}

/// 클라우드에만 있는 오디오를 받는다. 받은 로컬 경로를 돌려준다
pub fn download_audio(local: &dyn Local, remote: &dyn Remote, id: &str, progress: Progress) -> Result<PathBuf> {
    if !valid_id(id) {
        return Err(eyre!("잘못된 녹음 id입니다: {id}"));
    }
    let name = remote_audio_name(remote, id)?.ok_or_else(|| eyre!("클라우드에 이 녹음의 오디오가 아직 없습니다"))?;
    let dest = local.recording_dir(id).join(&name);
    remote.download(&format!("recordings/{id}/{name}"), &dest, progress)?;
    local.update_meta(id, &mut |m| {
        m["audioFile"] = Value::String(name.clone());
    })?;
    Ok(dest)
}

/// 클라우드에 올라간 오디오 파일 이름(audio.m4a 등). 아직 없으면 None
pub fn remote_audio_name(remote: &dyn Remote, id: &str) -> Result<Option<String>> {
    Ok(remote
        .list(&format!("recordings/{id}"))?
        .iter()
        .filter_map(|f| f.path.rsplit('/').next())
        .find(|n| n.starts_with("audio."))
        .map(str::to_string))
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

fn sync_recording(
    local: &dyn Local,
    remote: &dyn Remote,
    id: &str,
    files: &BTreeMap<String, RemoteFile>,
    rs: &mut RecordingState,
    report: &mut SyncReport,
    progress: Progress,
) -> Result<()> {
    let dir = local.recording_dir(id);
    let prefix = format!("recordings/{id}/");

    // 오디오: 클라우드에 없으면 올린다. 클라우드에만 있으면 필요할 때(download_audio) 받는다
    let mut remote_audio = files.keys().find(|n| n.starts_with("audio.")).cloned();
    let mut audio_uploaded = false;
    if remote_audio.is_none() {
        let local_meta = local.read_meta(id)?;
        if let Some(file) = local_meta.get("audioFile").and_then(|v| v.as_str()) {
            let source = dir.join(file);
            if source.is_file() {
                let upload = local.prepare_audio(id, &source)?;
                let name = format!("audio.{}", upload.ext);
                let result = remote.upload(&format!("{prefix}{name}"), &upload.path, progress);
                if upload.temporary {
                    let _ = fs::remove_file(&upload.path);
                }
                result?;
                report.uploaded += 1;
                remote_audio = Some(name);
                audio_uploaded = true;
            }
        }
    }

    sync_meta(local, remote, id, files.get("meta.json"), remote_audio.as_deref(), audio_uploaded, rs, report)?;

    for name in RESULT_FILES {
        let pulled = sync_file(remote, &dir.join(name), &format!("{prefix}{name}"), files.get(*name), rs.files.entry(name.to_string()).or_default(), report)?;
        if pulled && *name == "transcript.json" {
            local.transcript_arrived(id)?;
        }
    }

    sync_notes(local, remote, id, files, rs, report)
}

#[allow(clippy::too_many_arguments)]
fn sync_meta(
    local: &dyn Local,
    remote: &dyn Remote,
    id: &str,
    remote_file: Option<&RemoteFile>,
    remote_audio: Option<&str>,
    force_push: bool,
    rs: &mut RecordingState,
    report: &mut SyncReport,
) -> Result<()> {
    let path = format!("recordings/{id}/meta.json");
    let local_meta = local.read_meta(id)?;
    let local_hash = fingerprint(&meta::project(&local_meta));
    let last = rs.files.get("meta.json").cloned().unwrap_or_default();
    let local_changed = last.local != Some(local_hash) || force_push;
    let remote_changed = last.remote.as_deref() != remote_file.map(|f| f.version.as_str());
    // 나누는 필드를 마지막으로 고친 시각. 앱이 updatedAt을 채우지 않으면 meta.json 수정 시각으로 대신한다
    let local_time = time_field(&local_meta).unwrap_or_else(|| modified(&local.recording_dir(id).join("meta.json")));

    let push = |rs: &mut RecordingState, report: &mut SyncReport| -> Result<()> {
        let mut body = meta::project(&local_meta);
        let obj = body.as_object_mut().unwrap();
        if let Some(audio) = remote_audio {
            obj.insert("audio".into(), Value::String(audio.into()));
        }
        obj.insert("updatedAt".into(), Value::String(local_time.to_rfc3339()));
        obj.insert("updatedBy".into(), Value::String(local.device_id()));
        let version = remote.write(&path, &serde_json::to_vec_pretty(&body)?)?;
        rs.files.insert("meta.json".into(), FileState { local: Some(local_hash), remote: Some(version) });
        report.uploaded += 1;
        Ok(())
    };

    let Some(remote_file) = remote_file else { return push(rs, report) };
    if !remote_changed {
        return if local_changed { push(rs, report) } else { Ok(()) };
    }
    let remote_meta: Value = serde_json::from_slice(&remote.read(&remote_file.path)?).wrap_err("클라우드 meta.json을 읽지 못했습니다")?;
    if local_changed {
        // 둘 다 바뀌었으면 마지막에 고친 쪽이 이긴다
        let remote_time = time_field(&remote_meta).unwrap_or(remote_file.modified);
        if local_time > remote_time {
            return push(rs, report);
        }
    }
    let updated = local.update_meta(id, &mut |m| {
        meta::apply(m, &remote_meta);
        // 이 기기에 아직 오디오가 없으면 클라우드 오디오 이름을 따른다(다른 기기가 나중에 올린 경우)
        let has_audio = m.get("audioFile").and_then(|f| f.as_str()).is_some_and(|f| local.recording_dir(id).join(f).is_file());
        if let (false, Some(audio)) = (has_audio, remote_meta.get("audio").and_then(|a| a.as_str())) {
            m["audioFile"] = Value::String(audio.into());
        }
    })?;
    rs.files.insert(
        "meta.json".into(),
        FileState { local: Some(fingerprint(&meta::project(&updated))), remote: Some(remote_file.version.clone()) },
    );
    report.downloaded += 1;
    Ok(())
}

/// 한쪽만 바뀐 파일은 반대쪽에 복사하고, 둘 다 바뀌었으면 늦게 고친 쪽을 따른다. 받았으면 true
fn sync_file(
    remote: &dyn Remote,
    local_path: &Path,
    remote_path: &str,
    remote_file: Option<&RemoteFile>,
    last: &mut FileState,
    report: &mut SyncReport,
) -> Result<bool> {
    let local_bytes = fs::read(local_path).ok();
    let local_hash = local_bytes.as_deref().map(fnv);
    let remote_version = remote_file.map(|f| f.version.clone());
    let local_changed = local_hash != last.local;
    let remote_changed = remote_version != last.remote;
    if !local_changed && !remote_changed {
        return Ok(false);
    }
    let take_remote = match (local_changed, remote_changed) {
        (true, false) => false,
        (false, true) => true,
        // 둘 다 바뀜: 없는 쪽보다 있는 쪽, 둘 다 있으면 늦게 고친 쪽
        _ => match (&local_bytes, remote_file) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(_), Some(rf)) => rf.modified > modified(local_path),
        },
    };
    if take_remote {
        match remote_file {
            Some(rf) => {
                let bytes = remote.read(&rf.path)?;
                write_atomic(local_path, &bytes)?;
                *last = FileState { local: Some(fnv(&bytes)), remote: Some(rf.version.clone()) };
                report.downloaded += 1;
                return Ok(true);
            }
            None => {
                let _ = fs::remove_file(local_path);
                *last = FileState::default();
            }
        }
    } else {
        match local_bytes {
            Some(bytes) => {
                let version = remote.write(remote_path, &bytes)?;
                *last = FileState { local: local_hash, remote: Some(version) };
                report.uploaded += 1;
            }
            None => {
                remote.delete_file(remote_path)?;
                *last = FileState::default();
            }
        }
    }
    Ok(false)
}

/// 메모: 기기마다 자기 파일(notes/<기기 id>.json)만 쓰고, 화면에는 모든 기기 메모를 합쳐 둔다
fn sync_notes(
    local: &dyn Local,
    remote: &dyn Remote,
    id: &str,
    files: &BTreeMap<String, RemoteFile>,
    rs: &mut RecordingState,
    report: &mut SyncReport,
) -> Result<()> {
    let me = local.device_id();
    let mine_key = format!("notes/{me}.json");

    // 바뀐 다른 기기 메모 파일. None이면 그 기기 파일이 사라졌다
    let mut changed: BTreeMap<String, Option<NotesFile>> = BTreeMap::new();
    for (key, file) in files.iter().filter(|(k, _)| k.starts_with("notes/") && **k != mine_key) {
        let Some(device) = key.strip_prefix("notes/").and_then(|k| k.strip_suffix(".json")) else { continue };
        if rs.files.get(key).and_then(|s| s.remote.as_deref()) != Some(file.version.as_str()) {
            let parsed: NotesFile = serde_json::from_slice(&remote.read(&file.path)?).unwrap_or_default();
            changed.insert(device.to_string(), Some(parsed));
            rs.files.insert(key.clone(), FileState { local: None, remote: Some(file.version.clone()) });
            report.downloaded += 1;
        }
    }
    let vanished: Vec<String> = rs.files.keys().filter(|k| k.starts_with("notes/") && **k != mine_key && !files.contains_key(*k)).cloned().collect();
    for key in vanished {
        rs.files.remove(&key);
        if let Some(device) = key.strip_prefix("notes/").and_then(|k| k.strip_suffix(".json")) {
            changed.insert(device.to_string(), None);
        }
    }

    let mut mine: Vec<Value> = Vec::new();
    let mut others: BTreeSet<String> = BTreeSet::new();
    let deleted = &mut rs.deleted_notes;
    let last_others = rs.others_notes.clone();
    local.update_meta(id, &mut |m| {
        let mut notes: Vec<Value> = m.get("notes").and_then(|n| n.as_array()).cloned().unwrap_or_default();
        // 예전 메모(동기화 전)에는 id와 기기가 없다. 이 기기 메모로 본다
        for n in notes.iter_mut() {
            if n.get("id").and_then(|v| v.as_str()).is_none() {
                n["id"] = Value::String(uuid::Uuid::new_v4().to_string());
            }
            if n.get("device").and_then(|v| v.as_str()).is_none() {
                n["device"] = Value::String(me.clone());
            }
        }
        // 지난번에 있던 다른 기기 메모가 없어졌으면 이 기기에서 지운 것이다
        let now: BTreeSet<String> = notes.iter().filter(|n| device_of(n) != me).map(note_id).collect();
        deleted.extend(last_others.difference(&now).cloned());

        for (device, file) in &changed {
            notes.retain(|n| device_of(n) != *device);
            if let Some(file) = file {
                for n in &file.notes {
                    let mut n = n.clone();
                    if n.get("id").and_then(|v| v.as_str()).is_none() {
                        continue;
                    }
                    n["device"] = Value::String(device.clone());
                    notes.push(n);
                }
                let gone: BTreeSet<&String> = file.deleted.iter().collect();
                notes.retain(|n| !gone.contains(&note_id(n)));
            }
        }
        notes.retain(|n| !deleted.contains(&note_id(n)));
        notes.sort_by(|a, b| time_of(a).total_cmp(&time_of(b)));

        mine = notes.iter().filter(|n| device_of(n) == me).cloned().collect();
        others = notes.iter().filter(|n| device_of(n) != me).map(note_id).collect();
        m["notes"] = Value::Array(notes);
    })?;
    rs.others_notes = others;

    let body = NotesFile { notes: mine, deleted: rs.deleted_notes.iter().cloned().collect() };
    let bytes = serde_json::to_vec_pretty(&body)?;
    let hash = fnv(&bytes);
    let last = rs.files.get(&mine_key).cloned().unwrap_or_default();
    let remote_version = files.get(&mine_key).map(|f| f.version.clone());
    let empty = body.notes.is_empty() && body.deleted.is_empty();
    if (last.local != Some(hash) || last.remote != remote_version) && !(empty && remote_version.is_none()) {
        let version = remote.write(&format!("recordings/{id}/{mine_key}"), &bytes)?;
        rs.files.insert(mine_key, FileState { local: Some(hash), remote: Some(version) });
        report.uploaded += 1;
    }
    Ok(())
}

fn note_id(n: &Value) -> String {
    n.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn device_of(n: &Value) -> String {
    n.get("device").and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn time_of(n: &Value) -> f64 {
    n.get("time").and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// 번호가 겹치면 늦게 만든 녹음(만든 시각, 그다음 id 순)에 가장 큰 번호 + 1을 준다.
/// 모든 기기가 같은 규칙을 써서 같은 결과가 된다
fn renumber(local: &dyn Local, ids: &BTreeSet<String>) -> Result<()> {
    let mut all: Vec<(u64, DateTime<Utc>, String)> = Vec::new();
    for id in ids {
        let m = local.read_meta(id)?;
        let number = m.get("number").and_then(|n| n.as_u64()).unwrap_or(0);
        let created = m
            .get("createdAt")
            .and_then(|c| c.as_str())
            .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
            .map_or(DateTime::<Utc>::MIN_UTC, |t| t.with_timezone(&Utc));
        all.push((number, created, id.clone()));
    }
    let mut max = all.iter().map(|a| a.0).max().unwrap_or(0);
    let mut groups: BTreeMap<u64, Vec<(DateTime<Utc>, String)>> = BTreeMap::new();
    for (number, created, id) in all {
        if number > 0 {
            groups.entry(number).or_default().push((created, id));
        }
    }
    for (_, mut group) in groups.into_iter().filter(|(_, g)| g.len() > 1) {
        group.sort();
        for (_, id) in group.into_iter().skip(1) {
            if local.is_busy(&id) {
                continue;
            }
            max += 1;
            let n = max;
            local.update_meta(&id, &mut |m| m["number"] = json!(n))?;
        }
    }
    Ok(())
}

fn time_field(meta: &Value) -> Option<DateTime<Utc>> {
    let text = meta.get("updatedAt")?.as_str()?;
    Some(DateTime::parse_from_rfc3339(text).ok()?.with_timezone(&Utc))
}

fn modified(path: &Path) -> DateTime<Utc> {
    fs::metadata(path).and_then(|m| m.modified()).map(DateTime::from).unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
