# 기기 간 동기화

Mac 데스크톱과 iOS 앱이 녹음, 전사, 요약, 메모를 주고받는 방식입니다.
서버는 없고, 사용자가 이미 쓰는 클라우드에 파일로 저장합니다.

## 저장 위치

| 방식 | 저장 위치 | macOS | Windows | iOS |
|---|---|---|---|---|
| 이 기기에만 | 기기 안 | O | O (유일한 방식) | O |
| iCloud Drive | `iCloud Drive/sori` | O | X | O (Files에서 폴더 선택) |

- iCloud Drive는 운영체제가 동기화하는 폴더에 파일을 읽고 쓰는 방식(`FolderRemote`)입니다. sori 안에 로그인 화면이 없고, 기기의 Apple 계정을 그대로 씁니다
- 무료 Apple ID로 서명한 앱(AltStore, SideStore)은 CloudKit을 쓸 수 없어 폴더 방식으로 접근합니다
- Windows는 이 기기에만 저장합니다. 설정에 동기화 항목이 보이지 않습니다
- 2026-10-29 결정: OneDrive, Google Drive, Android는 넣지 않습니다. 엔진(`Remote`)은 그대로라 나중에 넣을 때는 저장소 구현만 더하면 됩니다

## 클라우드에 놓이는 파일

```
<sori 루트>/
  recordings/<id>/
    meta.json            제목, 번호, 날짜, 태그, 녹음한 곳 등 기기끼리 나누는 필드(`SHARED`, docs/organize.md)
    audio.m4a            압축한 오디오. 필요할 때만 받습니다
    transcript.json      전사. 데스크톱만 씁니다
    summaries.json       요약 목록. 데스크톱만 씁니다
    summary.json         최신 요약(MCP, Raycast가 읽음)
    notes/<기기 id>.json 그 기기에서 남긴 메모와, 그 기기가 지운 메모 id
  deleted/<id>.json      지운 녹음 표시(tombstone). 다른 기기도 지웁니다
```

녹음 중인 트랙(`mic.wav`, `system.wav`)과 원본 WAV는 올리지 않습니다.

## 충돌을 막는 규칙

여러 기기가 같은 파일을 동시에 고치면 충돌이 납니다. 파일마다 쓰는 쪽을 정해 병합 로직을 최소로 했습니다.

| 파일 | 쓰는 쪽 | 둘 다 바뀌었을 때 |
|---|---|---|
| `meta.json` | 모든 기기 | 마지막에 고친 쪽(`updatedAt`)이 이깁니다 |
| `transcript.json`, `summaries.json`, `summary.json` | 처리한 데스크톱 | 마지막에 고친 쪽이 이깁니다 |
| `notes/<기기 id>.json` | 그 기기만 | 충돌 없음. 화면에는 모든 기기 메모를 합쳐 보여 줍니다 |
| `audio.*` | 녹음하거나 가져온 기기 | 한 번 올리면 바꾸지 않습니다 |

녹음 번호(#12)는 남은 녹음의 가장 큰 번호 + 1입니다(데스크톱 `src-tauri/src/store.rs` `next_number`, iPhone `mobile/src-tauri/src/store.rs` `create_recording`). 맨 뒤 녹음을 지우면 그 번호부터 다시 쓰고, 중간을 지우면 빈칸으로 둡니다. 남은 녹음의 번호는 바뀌지 않습니다. 0.9.1까지는 `last-number` 파일로 지운 번호를 다시 쓰지 않았는데, 테스트 녹음 25개를 지우자 #5 다음이 #117이 되어 0.9.2에서 뺐습니다. 남아 있는 `last-number` 파일은 읽지 않습니다.
번호는 기기마다 따로 매기므로 오프라인에서 같은 번호가 생길 수 있습니다.
동기화할 때 번호가 겹치면 늦게 만든 녹음(만든 시각, 그다음 id 순)에 가장 큰 번호 + 1을 줍니다. 모든 기기가 같은 규칙을 쓰므로 결과가 같아집니다.

## 동기화 순서

`crates/sori-sync/src/engine.rs`의 `sync`가 한 번에 다음을 합니다.

1. 클라우드의 `deleted/`를 읽어 지워진 녹음을 이 기기에서도 지웁니다
2. 이 기기에서 지운 녹음(지난번 동기화 상태에는 있는데 지금은 없음)을 클라우드에서 지우고 tombstone을 남깁니다
3. 녹음마다 파일을 비교합니다. 지난번 동기화 때 기록한 지문(로컬 내용 해시, 클라우드 버전)과 비교해 바뀐 쪽을 반대쪽에 복사합니다. 둘 다 바뀌었으면 위 표의 규칙을 따릅니다
4. 오디오가 클라우드에 없으면 압축해 올립니다. 클라우드에만 있으면 받지 않고 "받기 전" 상태로 둡니다
5. 겹친 번호를 고칩니다

녹음 중이거나 처리 중인 녹음은 건너뜁니다. 동기화 상태는 기기의 데이터 폴더 `sync-state.json`에 둡니다.

## 오디오 압축

| 원본 | 올리는 형식 | 1시간 크기 |
|---|---|---|
| 압축 형식(m4a, mp3, aac, ogg, opus, flac 등) | 그대로 | 원본과 같음 |
| WAV 등 | AAC m4a, 모노 48kbps (macOS `afconvert`) | 약 22MB |

모바일 앱은 처음부터 AAC m4a로 녹음합니다. 데스크톱의 디코더(symphonia)는 AAC를 읽습니다.

## 코드 위치

| 위치 | 역할 |
|---|---|
| `crates/sori-sync/src/remote.rs` `Remote` | 클라우드 공통 동작(목록, 읽기, 쓰기, 지우기) |
| `crates/sori-sync/src/folder.rs` `FolderRemote` | 폴더 방식(iCloud Drive) |
| `crates/sori-sync/src/engine.rs` `sync`, `download_audio` | 동기화 순서와 충돌 규칙 |
| `crates/sori-sync/src/local.rs` `Local` | 기기 저장소에 접근하는 방법. 데스크톱과 모바일이 각자 구현 |
| `src-tauri/src/sync.rs` `default_folder`, `remote_for` | iCloud Drive 기본 폴더 찾기 |
| `src-tauri/src/sync.rs` `start`, `run_once`, `DesktopLocal` | 데스크톱 연결: 30초마다와 변경 직후(3초 모아서) 실행. "지금 동기화"는 모으지 않고 바로, 오디오 압축, 화면 알림, 다른 기기 녹음 자동 처리 |
| `src-tauri/src/store.rs` `update_raw`, `create_raw`, `Recording::updated_at` | 엔진이 meta.json을 고치는 통로. 앱이 나누는 필드를 바꾸면 `updatedAt`을 새로 적는다 |
| `src/components/SyncSettings.tsx` | 설정 화면의 동기화 항목, 사이드바 상태 |

## 진행 상황

| 단계 | 상태 |
|---|---|
| 동기화 엔진, iCloud Drive(폴더), 데스크톱 연결 | 0.5.0 |
| OneDrive(폴더) | 0.5.1에 넣었다가 0.5.2에서 뺌 |
| iOS 앱 (Tauri 2, AltStore로 직접 설치, docs/mobile.md) | 0.6.0 |

## iPhone iCloud가 바뀐 파일을 늦게 받는 문제

iOS iCloud Drive는 한 번 내려받은 파일이 클라우드에서 바뀌어도 바로 받지 않고 예전 내용을 둡니다(`ubiquitousItemDownloadingStatus`가 `.current`가 아닌 `.downloaded`). 동기화 엔진은 폴더의 파일 수정 시각과 크기로 클라우드 쪽 변경을 알아채므로(`crates/sori-sync/src/folder.rs` `version_of`), 내려오기 전에는 바뀐 게 없다고 봅니다. 0.9.5까지 Mac에서 고친 태그가 iPhone에 한 시간 넘게 반영되지 않았습니다(Mac은 00:54에 올렸고 iPhone은 계속 예전 태그).

0.9.6부터 iPhone은 동기화할 때마다 먼저 폴더를 훑어 최신이 아닌 작은 파일(meta.json, 전사, 요약, 메모)을 받아 옵니다(`SoriNativePlugin.swift` `refreshIcloud`, `mobile/src-tauri/src/sync.rs` `remote`). 오디오는 크므로 건드리지 않고 필요할 때 받습니다. 최대 15초 기다립니다.

