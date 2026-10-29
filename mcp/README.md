# sori MCP 서버

Claude Code, Claude 데스크톱 같은 MCP 클라이언트에서 sori 녹음을 찾고 읽습니다. 읽기 전용이라 녹음을 바꾸거나 지우지 않습니다. sori가 꺼져 있어도 됩니다.

| 도구 | 하는 일 |
|---|---|
| `list_recordings` | 최신순 녹음 목록(id, 제목, 날짜, 길이, 요약 여부) |
| `search_recordings` | 제목, 요약, 전사에서 낱말 찾기. 띄어쓰기와 대소문자는 무시 |
| `get_recording` | 녹음 하나의 요약, 메모, 전사를 마크다운으로 |

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

- "sori에서 지난주 배포 회의 찾아서 결정 사항 정리해 줘"
- "sori 녹음 중에 민지가 말한 걱정거리 모아서 이슈로 만들어 줘"
- "오늘 회의 요약을 PR 설명에 넣어 줘"

## 동작 방식

sori가 저장한 파일을 직접 읽습니다. macOS `~/Library/Application Support/com.nobel6018.sori/recordings/`, Windows `%APPDATA%\com.nobel6018.sori\recordings\`입니다. 다른 곳을 읽게 하려면 `SORI_DATA_DIR` 환경 변수에 데이터 폴더를 줍니다. 형식은 `src-tauri/src/store.rs`와 같아야 하고, Raycast 확장(`raycast/src/store.ts`)과 같은 방식입니다.

전사를 보낼지는 클라이언트(Claude)가 정합니다. MCP 서버는 이 컴퓨터 안에서만 돌지만, 읽은 내용은 Claude 대화에 들어가 Anthropic으로 갑니다.
