# sori MCP 서버

Claude Code, Claude 데스크톱 같은 MCP 클라이언트에서 sori 녹음을 찾고 읽고, 받아 적기와 요약을 sori 앱에 맡깁니다. 녹음을 지우지는 않습니다. 찾기와 읽기는 sori가 꺼져 있어도 됩니다.

| 도구 | 하는 일 | 앱 필요 |
|---|---|---|
| `list_recordings` | 최신순 녹음 목록(번호, 제목, 날짜, 길이, 상태, 태그, 요약 여부). 날짜 범위(`since`, `until`), 태그(상위 태그는 하위 포함), 상태로 거른다 | 아니요 |
| `search_recordings` | 제목, 요약, 전사에서 낱말 찾기. 띄어쓰기와 대소문자는 무시 | 아니요 |
| `get_recording` | 상태와 진행률, 화자별 발화 시간, 요약, 메모, 전사. 긴 회의는 `from`, `to`(시각), `speakers`(화자)로 일부만 읽고 `max_chars`로 줄인다. 잘렸으면 이어 읽을 시각을 알려 준다 | 아니요 |
| `get_todos` | 요약에서 AI가 뽑은 할 일 후보(id, 내용, 기한, 미리 알림에 넣었는지) | 아니요 |
| `process_recording` | 받아 적기(전사, 화자 분리)를 맡긴다. 화자 수, 언어, 요약 여부, 요약 템플릿을 줄 수 있다. 이미 전사가 있으면 덮어쓴다 | 0.10.0~ (템플릿은 0.11.0~) |
| `summarize_recording` | 받아 적은 녹음을 다시 요약한다. 템플릿을 고를 수 있다 | 0.10.0~ (템플릿은 0.11.0~) |
| `set_speaker_names` | "화자 1, 2"에 이름을 붙인다. 앱의 이름 바꾸기와 같아 목소리 기억도 따른다 | 0.11.0~ |
| `add_todos_to_reminders` | 고른 할 일을 macOS 미리 알림에 넣는다 | 0.11.0~ |
| `import_audio` | 오디오, 영상 파일을 가져와 바로 받아 적게 한다. 화자 수, 언어도 줄 수 있다 | 0.11.0~ |

## 설치

```sh
cd mcp
npm install
npm run build
```

Claude Code에 등록합니다.

```sh
claude mcp add sori --scope user -- node "$PWD/dist/index.js"
```

Claude 데스크톱은 설정 파일(`claude_desktop_config.json`)의 `mcpServers`에 넣습니다.

```json
{
  "mcpServers": {
    "sori": { "command": "node", "args": ["/절대/경로/sori/mcp/dist/index.js"] }
  }
}
```

## 쓰는 예

- "sori 12번 녹음 요약해서 회의록으로 정리해 줘"
- "sori에서 지난주 배포 회의 찾아서 결정 사항 정리해 줘"
- "sori 녹음 중에 민지가 말한 걱정거리 모아서 이슈로 만들어 줘"
- "오늘 회의 요약을 PR 설명에 넣어 줘"
- "sori 16번 3명이고 한국어야. 받아 적고 요약해 줘"
- "16번 전사 20분부터 30분까지만 보여 줘"
- "16번 화자 이름을 대화 내용 보고 붙여 줘"
- "16번 할 일 중 내 것만 미리 알림에 넣어 줘"
- "~/Downloads/팀즈 회의.mp4 가져와서 4명, 한국어로 받아 적어 줘"

## 동작 방식

sori가 저장한 파일을 직접 읽습니다. macOS `~/Library/Application Support/com.nobel6018.sori/recordings/`, Windows `%APPDATA%\com.nobel6018.sori\recordings\`입니다. 다른 곳을 읽게 하려면 `SORI_DATA_DIR` 환경 변수에 데이터 폴더를 줍니다. 형식은 `src-tauri/src/store.rs`와 같아야 하고, Raycast 확장(`raycast/src/store.ts`)과 같은 방식입니다.

### 받아 적기와 요약

MCP 서버는 앱 데이터를 직접 고치지 않습니다. 엔진(whisper, 화자 분리)과 요약 설정이 앱에 있고, 앱과 같은 파일을 동시에 쓰면 충돌하기 때문입니다. 대신 `sori://process`, `summarize`, `speakers`, `reminders`, `import` 링크를 열어 앱에 맡기고, 결과는 파일이 바뀌는 것으로 확인합니다. 앱이 꺼져 있으면 켜지고, 가져오기 말고는 창이 앞으로 오지 않습니다.

- 링크에는 데이터 폴더의 `link-token` 값을 붙입니다. 웹 페이지가 같은 링크로 처리를 시키지 못하게 하는 값이고, 도구 결과에는 나오지 않습니다(`docs/shortcuts-and-deep-links.md`)
- 링크를 연 뒤 45초 안에 녹음 상태가 "처리 중"으로 바뀌는지 봅니다. 바뀌지 않으면 앱이 요청을 받지 못한 것으로 알립니다(오래된 앱 버전 등)
- 1시간 녹음은 몇 분 걸립니다. 기본은 시작만 확인하고 돌아오고, `wait_seconds`를 주면 그만큼 끝나길 기다립니다. 요약은 `summary.json`의 만든 시각이 바뀌면 끝난 것으로 봅니다
- 진행률: 앱(0.11.0~)이 처리와 요약 중에 데이터 폴더 `activity/<녹음 id>.json`에 단계(기다림, 디코딩, 화자 분리, 받아 적기, 요약)와 진행률을 적습니다(`src-tauri/src/activity.rs`). `get_recording` 상태 줄에 "화자 나누는 중 42%"처럼 나오고, `wait_seconds`로 기다리는 동안 클라이언트가 진행 토큰을 보냈으면 MCP 진행 알림으로도 보냅니다. 10분 넘게 안 바뀐 파일은 비정상 종료로 남은 것으로 보고 무시합니다
- 가져오기는 `process=0`으로 가져온 뒤 처리 링크를 따로 보냅니다. 앱의 가져오기 자동 처리는 기본 설정만 쓰기 때문이고, `process=0`이면 앱이 자동 처리도 하지 않습니다
- 녹음 중이거나 이미 처리 중인 녹음은 요청하지 않습니다. 앱도 같은 조건으로 한 번 더 막습니다

전사를 보낼지는 클라이언트(Claude)가 정합니다. MCP 서버는 이 컴퓨터 안에서만 돌지만, 읽은 내용은 Claude 대화에 들어가 Anthropic으로 갑니다.
