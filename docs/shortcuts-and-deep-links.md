# 단축키, sori:// 링크, 파일 열기, 빠른 찾기

앱 밖에서 들어오는 요청(전역 단축키, `sori://` 링크, Finder의 "다음으로 열기")은 모두 같은 길로 화면에 전달됩니다.

```
전역 단축키 ─┐
sori:// 링크 ─┼─> actions::dispatch ──(app-action 이벤트)──> App.handleAction ──> sendCommand ──> Recorder, RecordingView
파일 열기 ────┘
앱 안 단축키 ─────────────────────────────────────────────> App.runCommand ───┘
```

녹음 상태와 처리 화면은 프론트가 들고 있습니다. 그래서 Rust는 할 일을 정해 넘기기만 하고, 실제 동작은 화면의 버튼과 같은 함수를 부릅니다.

| 위치 | 하는 일 |
|---|---|
| `src-tauri/src/actions.rs` `parse_url`, `dispatch`, `apply_shortcuts` | 링크 해석, 전역 단축키 등록, 화면이 준비되기 전 요청 보관 |
| `src-tauri/src/lib.rs` `run`, `frontend_ready`, `setup_deep_links` | 플러그인 등록, 파일 열기(`RunEvent::Opened`, Windows 실행 인자) |
| `src-tauri/src/search.rs` `search` | 제목, 요약, 전사 검색. 전사는 수정 시각으로 캐시 |
| `src/shortcuts.ts` `SHORTCUTS`, `matchShortcut`, `useCommands` | 앱 안 단축키 목록과 컴포넌트 사이 명령 전달 |
| `src/App.tsx` `handleAction`, `runCommand` | 요청을 화면 동작으로 바꿈 |
| `src/components/QuickOpen.tsx` | 빠른 찾기 창 (목록, 미리 보기, 마크다운 복사) |
| `src/markdown.ts` `recordingMarkdown` | 복사용 마크다운. `raycast/src/markdown.ts`와 형식이 같아야 함 |

## 전역 단축키

sori가 앞에 없어도 동작합니다. 설정에서 바꾸고, Backspace로 끕니다.

| 동작 | macOS | Windows |
|---|---|---|
| 녹음 시작/멈춤 | ⌥⇧⌘R | Ctrl+Alt+Shift+R |
| sori 창 앞으로 | ⌥⇧⌘S | Ctrl+Alt+Shift+S |

녹음 토글은 창을 앞으로 가져오지 않습니다. 회의 앱을 보는 중에 누르는 키라서입니다. 다른 앱이 이미 쓰는 조합이면 등록이 실패하고, 설정은 저장되지 않은 채 이유를 보여 줍니다.

## 앱 안 단축키

⌘/(Windows는 Ctrl+/)로 앱 안에서 전체 목록을 봅니다. 목록의 원본은 `src/shortcuts.ts`의 `SHORTCUTS`입니다.

- 글자 입력칸에 커서가 있으면 Space, 화살표, ⌘⌫ 같은 키는 입력칸의 원래 동작을 따릅니다. ⌘K, ⌘O, ⌘↩처럼 입력칸에서 쓰지 않는 조합만 동작합니다
- 한글 조합 중에 누른 키는 무시합니다. 조합 중 Enter와 Space는 글자를 완성하는 키입니다
- ⌘R은 쓰지 않습니다. 웹뷰가 새로고침으로 받을 수 있어, 녹음 중이면 화면 상태를 잃습니다

## sori:// 링크

| 링크 | 동작 | 창을 앞으로 |
|---|---|---|
| `sori://record/toggle?token=<토큰>` | 녹음 시작/멈춤 | 아니요 |
| `sori://record/start?token=..`, `sori://record/stop?token=..` | 녹음 시작, 멈춤 | 아니요 |
| `sori://show` | 창 앞으로 | 예 |
| `sori://open?id=<녹음 id>&t=<초>` | 녹음을 열고 그 시각으로 이동. `t`는 생략 가능 | 예 |
| `sori://import?path=<경로>&path=<경로>&process=1&token=<토큰>` | 파일을 가져옴. `process=1`이면 기본 설정으로 바로 처리 | 예 |
| `sori://search?q=<검색어>` | 빠른 찾기 창을 그 검색어로 엶 | 예 |

링크로 앱을 처음 띄우면 화면이 준비되기 전에 링크가 도착합니다. Rust가 요청을 보관해 두었다가, 화면이 이벤트 구독을 마치고 `frontend_ready`를 부를 때 넘깁니다.

macOS에서는 설치된 앱(.app)만 링크를 받습니다. `pnpm tauri dev`로는 시험할 수 없습니다. `pnpm tauri build --debug --bundles app`으로 묶은 뒤 `open -a <경로>/sori.app "sori://search?q=회의"`로 시험합니다.

### 링크 토큰

웹 페이지도 `sori://` 링크를 열 수 있습니다. 브라우저의 "sori를 여시겠습니까?" 확인창에서 "항상 허용"을 고른 사용자라면, 아무 웹 페이지나 몰래 녹음을 켤 수 있게 됩니다. 그래서 마이크를 켜는 링크(`record/*`)와 이 컴퓨터의 파일을 읽는 링크(`import`)는 `token`을 함께 보내야 합니다(`actions::needs_token`).

- sori는 처음 실행할 때 데이터 폴더에 `link-token` 파일을 만듭니다(macOS `~/Library/Application Support/com.nobel6018.sori/link-token`, 권한 600). 이 컴퓨터의 사용자 권한으로 도는 프로그램만 읽을 수 있고, 웹 페이지는 읽지 못합니다
- 토큰이 없거나 틀리면 요청을 버리고 창을 띄워 막았다고 알립니다
- 열기, 검색, 창 앞으로는 토큰 없이 받습니다. 화면을 보여 줄 뿐 데이터를 바꾸거나 내보내지 않습니다
- 가져오기 경로는 절대 경로의 실제 오디오 파일만 받습니다. Windows 네트워크 경로(`\\server\share`)는 여는 순간 로그인 정보가 그 서버로 가므로 거부합니다(`actions::audio_files`)

링크로 녹음을 지우거나 파일을 밖으로 보내는 동작은 만들지 않았습니다.

## 녹음 파일 바로 열기

- macOS: Finder에서 오디오 파일을 우클릭해 "다음으로 열기 > sori"를 고르거나, Dock의 sori 아이콘에 끌어다 놓습니다. sori는 보조 앱(`LSHandlerRank: Alternate`)으로만 등록해, mp3 같은 파일의 기본 앱을 바꾸지 않습니다(`src-tauri/tauri.macos.conf.json`)
- Windows: "연결 프로그램 > 다른 앱 선택"에서 sori.exe를 고르면 됩니다. Tauri의 Windows 설치 파일은 파일 연결을 넣으면 확장자의 기본 앱을 덮어쓰므로, 설치 파일에는 넣지 않았습니다
- 앱 창에 파일을 끌어다 놓아도 됩니다

녹음 화면의 "Finder에서 보기"(⇧⌘E)는 sori가 보관한 오디오 사본을 보여 줍니다.

## 빠른 찾기 (⌘K, ⌘F)

검색어가 없으면 모든 녹음을 최신순으로 보여 주고, 고른 녹음의 요약을 오른쪽에 미리 보여 줍니다. 검색어를 넣으면 제목, 요약, 전사에서 찾고, 찾은 문장 근처의 전사를 미리 보여 줍니다. 대소문자와 띄어쓰기는 무시합니다("금요일 까지"로 "금요일까지"를 찾음).

| 키 | 동작 |
|---|---|
| ↑ ↓ | 고르기 |
| ↩ | 열기. 문장을 골랐으면 그 시각으로 이동 |
| ⇧⌘C | 요약과 전사를 한 문서로 복사 |
| ⇧⌘S | 요약만 복사 |
| ⇧⌘T | 전사만 복사 |
| Esc | 닫기 |

복사하는 마크다운은 제목, 날짜, 길이, 참석자, 요약, 전사 순입니다. 요약의 제목 단계는 한 단계씩 내려 문서 제목 아래에 들어가게 합니다.
