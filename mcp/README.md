# sori MCP 서버

Claude Code, Claude 데스크톱 같은 MCP 클라이언트에서 sori 녹음을 찾고 읽고, 받아 적기와 요약을 sori 앱에 맡깁니다. 녹음을 지우지는 않습니다. 찾기와 읽기는 sori가 꺼져 있어도 됩니다.

| 도구 | 하는 일 |
|---|---|
| `list_recordings` | 최신순 녹음 목록(id, 제목, 날짜, 길이, 요약 여부) |
| `search_recordings` | 제목, 요약, 전사에서 낱말 찾기. 띄어쓰기와 대소문자는 무시 |
| `get_recording` | 녹음 하나의 상태, 요약, 메모, 전사를 마크다운으로. 번호(`12`, `#12`)나 id로 부른다 |
| `process_recording` | sori 앱에 받아 적기(전사, 화자 분리)를 맡긴다. 화자 수, 언어, 요약 여부를 줄 수 있다. 이미 전사가 있으면 덮어쓴다 |
| `summarize_recording` | 받아 적은 녹음을 다시 요약한다. 요약 방식과 템플릿은 앱 설정을 따른다 |

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

## 동작 방식

sori가 저장한 파일을 직접 읽습니다. macOS `~/Library/Application Support/com.nobel6018.sori/recordings/`, Windows `%APPDATA%\com.nobel6018.sori\recordings\`입니다. 다른 곳을 읽게 하려면 `SORI_DATA_DIR` 환경 변수에 데이터 폴더를 줍니다. 형식은 `src-tauri/src/store.rs`와 같아야 하고, Raycast 확장(`raycast/src/store.ts`)과 같은 방식입니다.

### 받아 적기와 요약

MCP 서버는 받아 적지 않습니다. 엔진(whisper, 화자 분리)과 요약 설정이 앱에 있어서, `sori://process`, `sori://summarize` 링크를 열어 앱에 맡깁니다(sori 0.10.0 이상). 앱이 꺼져 있으면 켜지고, 창은 앞으로 오지 않습니다.

- 링크에는 데이터 폴더의 `link-token` 값을 붙입니다. 웹 페이지가 같은 링크로 처리를 시키지 못하게 하는 값이고, 도구 결과에는 나오지 않습니다(`docs/shortcuts-and-deep-links.md`)
- 링크를 연 뒤 45초 안에 녹음 상태가 "처리 중"으로 바뀌는지 봅니다. 바뀌지 않으면 앱이 요청을 받지 못한 것으로 알립니다(오래된 앱 버전 등)
- 1시간 녹음은 몇 분 걸립니다. 기본은 시작만 확인하고 돌아오고, `wait_seconds`를 주면 그만큼 끝나길 기다립니다. 요약은 `summary.json`의 만든 시각이 바뀌면 끝난 것으로 봅니다
- 녹음 중이거나 이미 처리 중인 녹음은 요청하지 않습니다. 앱도 같은 조건으로 한 번 더 막습니다

전사를 보낼지는 클라이언트(Claude)가 정합니다. MCP 서버는 이 컴퓨터 안에서만 돌지만, 읽은 내용은 Claude 대화에 들어가 Anthropic으로 갑니다.
