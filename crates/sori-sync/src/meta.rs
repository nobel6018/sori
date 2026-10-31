//! meta.json 중 기기끼리 나누는 필드와 메모 파일 형식

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 기기끼리 나누는 meta.json 필드. 상태(처리 중 등), 오류, 트랙, 목소리 id는 기기마다 다르므로 뺐다.
/// tags, tagsEditedAt(사람이 태그를 고친 시각), autoTaggedAt(AI가 분류한 시각), location은 0.8.0에,
/// event(겹치는 캘린더 일정), titleEditedAt(사람이 제목을 고친 시각)은 0.9.0에 더했다(docs/organize.md).
/// 여기 없는 필드는 올리지 않으므로, 0.7.x 앱이 녹음을 고치면 이 필드들이 클라우드에서 빠진다. 기기들을 함께 올린다
pub const SHARED: &[&str] = &[
    "id",
    "number",
    "title",
    "createdAt",
    "sourceName",
    "duration",
    "speakerNames",
    "numSpeakers",
    "languages",
    "diarize",
    "tags",
    "tagsEditedAt",
    "autoTaggedAt",
    "location",
    "event",
    "titleEditedAt",
];

/// 로컬 meta.json에서 나누는 필드만 뽑는다
pub fn project(local: &Value) -> Value {
    let mut out = Map::new();
    for key in SHARED {
        if let Some(v) = local.get(*key).filter(|v| !v.is_null()) {
            out.insert((*key).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

/// 클라우드 meta.json의 나누는 필드를 로컬 meta에 덮어쓴다
pub fn apply(local: &mut Value, remote: &Value) {
    let Some(obj) = local.as_object_mut() else { return };
    for key in SHARED.iter().filter(|k| **k != "id") {
        match remote.get(*key) {
            Some(v) => obj.insert((*key).to_string(), v.clone()),
            None => obj.remove(*key),
        };
    }
    // 고친 시각도 따라가야 다음 충돌 비교에서 이 값을 쓴다
    if let Some(t) = remote.get("updatedAt") {
        obj.insert("updatedAt".into(), t.clone());
    }
}

/// 다른 기기에서 온 녹음의 로컬 meta.json. 데스크톱 Recording이 꼭 요구하는 필드를 채운다
pub fn new_local(remote: &Value, has_transcript: bool) -> Value {
    let mut meta = project(remote);
    let obj = meta.as_object_mut().unwrap();
    obj.entry("title").or_insert_with(|| Value::String("녹음".into()));
    obj.entry("sourceName").or_insert_with(|| Value::String(String::new()));
    obj.entry("createdAt").or_insert_with(|| Value::String(chrono::Utc::now().to_rfc3339()));
    let audio = remote.get("audio").and_then(|a| a.as_str()).unwrap_or("audio.m4a");
    obj.insert("audioFile".into(), Value::String(audio.into()));
    obj.insert("status".into(), Value::String(if has_transcript { "done" } else { "new" }.into()));
    obj.insert("notes".into(), Value::Array(Vec::new()));
    meta
}

/// 내용이 같으면 같은 값. 객체 키 순서에 상관없이 계산한다
pub fn fingerprint(value: &Value) -> u64 {
    let mut text = String::new();
    canonical(value, &mut text);
    fnv(text.as_bytes())
}

fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for k in keys {
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                canonical(&map[k], out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for v in items {
                canonical(v, out);
                out.push(',');
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// FNV-1a 64. 앱 버전이 바뀌어도 같은 값을 내야 해서(동기화 상태 파일에 저장) 표준 해시를 쓰지 않는다
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `notes/<기기 id>.json`. 그 기기가 남긴 메모와, 그 기기가 지운 메모 id(다른 기기 메모 포함)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesFile {
    #[serde(default)]
    pub notes: Vec<Value>,
    #[serde(default)]
    pub deleted: Vec<String>,
}
