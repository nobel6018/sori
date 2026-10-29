# 기기 간 동기화

데스크톱(macOS, Windows)과 모바일(iOS, Android) 앱이 녹음, 전사, 요약, 메모를 주고받는 방식입니다.
서버는 없고, 사용자가 이미 쓰는 클라우드에 파일로 저장합니다.

## 저장 위치

| 방식 | 저장 위치 | 데스크톱 | iOS | Android |
|---|---|---|---|---|
| 이 기기에만 | 기기 안 | O | O | O |
| iCloud Drive | `iCloud Drive/sori` | O (Windows는 iCloud for Windows) | O (Files에서 폴더 선택) | X |
| OneDrive | `OneDrive/sori` | O (OneDrive 앱) | O (Files에서 폴더 선택) | X |

- 둘 다 운영체제의 클라우드 앱이 동기화하는 폴더에 파일을 읽고 쓰는 방식(`FolderRemote`)입니다. sori 안에 로그인 화면이 없고, 각 클라우드 앱의 계정을 그대로 씁니다
- iCloud는 무료 Apple ID로 서명한 앱(AltStore, SideStore)에서 CloudKit을 쓸 수 없어 폴더 방식으로 접근합니다
- OneDrive 기본 폴더: macOS는 `~/Library/CloudStorage/OneDrive-Personal`(개인 계정)을 먼저, 없으면 `OneDrive-회사이름`을 고릅니다. Windows는 OneDrive 앱이 적어 둔 환경 변수(`OneDriveConsumer`, `OneDrive`, `OneDriveCommercial`)를 봅니다. 설정에서 다른 폴더를 고를 수 있습니다
- Android는 Files 연동이 클라우드 폴더 접근을 지원하지 않아, Android 앱을 만들 때 OneDrive, Google Drive API 방식을 다시 검토합니다(2026-10-29 사용자 결정으로 보류)

## 클라우드에 놓이는 파일

```
<sori 루트>/
  recordings/<id>/
    meta.json            제목, 번호, 날짜 등 기기끼리 나누는 필드(`SharedMeta`)
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

녹음 번호(#12)는 기기마다 따로 매기므로 오프라인에서 같은 번호가 생길 수 있습니다.
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
| WAV 등 (macOS) | AAC m4a, 모노 48kbps (`afconvert`) | 약 22MB |
| WAV 등 (Windows) | FLAC, 모노 16kHz (`flacenc`) | 약 50MB |

모바일 앱은 처음부터 AAC m4a로 녹음합니다. 데스크톱의 디코더(symphonia)는 AAC, FLAC을 모두 읽습니다.

## 코드 위치

| 위치 | 역할 |
|---|---|
| `crates/sori-sync/src/remote.rs` `Remote` | 클라우드 공통 동작(목록, 읽기, 쓰기, 지우기) |
| `crates/sori-sync/src/folder.rs` `FolderRemote` | 폴더 방식(iCloud Drive, OneDrive) |
| `crates/sori-sync/src/engine.rs` `sync`, `download_audio` | 동기화 순서와 충돌 규칙 |
| `crates/sori-sync/src/local.rs` `Local` | 기기 저장소에 접근하는 방법. 데스크톱과 모바일이 각자 구현 |
| `src-tauri/src/sync.rs` `default_folder`, `remote_for` | 클라우드별 기본 폴더 찾기 |
| `src-tauri/src/sync.rs` `start`, `run_once`, `DesktopLocal` | 데스크톱 연결: 2분마다와 변경 직후(3초 모아서) 실행, 오디오 압축, 화면 알림, 다른 기기 녹음 자동 처리 |
| `src-tauri/src/store.rs` `update_raw`, `create_raw`, `Recording::updated_at` | 엔진이 meta.json을 고치는 통로. 앱이 나누는 필드를 바꾸면 `updatedAt`을 새로 적는다 |
| `src/components/SyncSettings.tsx` | 설정 화면의 동기화 항목, 사이드바 상태 |

## 진행 상황

| 단계 | 상태 |
|---|---|
| 동기화 엔진, iCloud Drive(폴더), 데스크톱 연결 | 0.5.0 |
| OneDrive(폴더) | 0.5.1 |
| iOS 앱 (Tauri 2, AltStore로 직접 설치) | 준비 중 |
| Android 앱, API 방식 클라우드 | 보류 |

## 알려진 함정

- flacenc는 FLAC 헤더(STREAMINFO)의 최소 블록 크기에 마지막 블록 길이를 적는다. symphonia는 최소와 최대가 다르면 파일을 읽지 못해(`end of stream`) `encode_flac`이 헤더를 고쳐 쓴다
