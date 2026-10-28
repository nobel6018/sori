# sori Raycast 확장

sori 녹음을 Raycast에서 찾고, 복사하고, 녹음을 켜고 끕니다. macOS 전용입니다.

| 명령 | 하는 일 |
|---|---|
| Search Transcripts | 모든 녹음의 제목, 요약, 전사에서 찾습니다. ↩는 sori에서 그 시각으로 열고, ⇧⌘C는 요약과 전사를 마크다운으로 복사합니다 |
| Recent Recordings | 최근 녹음 목록. 요약과 전사를 미리 보고 복사합니다 |
| Toggle Recording | 녹음을 시작하거나 멈춥니다. sori 창은 앞으로 나오지 않습니다 |
| Import Selected Audio | Finder에서 고른 오디오를 sori로 가져와 바로 받아 적습니다 |
| Recording Status | 메뉴 막대에 녹음 경과 시간을 보여 주고, 눌러서 멈춥니다 |

## 설치

sori 앱(0.3.0 이상)이 먼저 설치돼 있어야 합니다. 스토어에 올리지 않았으므로 직접 불러옵니다.

```sh
cd raycast
npm install
npm run dev   # Raycast에 확장을 불러옵니다. 한 번 불러오면 dev를 꺼도 남아 있습니다
```

## 동작 방식

- 읽기: sori가 저장한 파일(`~/Library/Application Support/com.nobel6018.sori/recordings/`)을 직접 읽습니다. sori가 꺼져 있어도 검색과 복사가 됩니다(`src/store.ts`)
- 동작: 녹음, 열기, 가져오기는 `open -g sori://...`로 sori에 넘깁니다. 녹음과 가져오기 링크에는 sori 데이터 폴더의 `link-token`을 붙입니다. 웹 페이지가 녹음을 켜지 못하게 하는 장치라, sori를 한 번도 실행하지 않았으면 이 두 명령은 동작하지 않습니다. 링크 목록은 [../docs/shortcuts-and-deep-links.md](../docs/shortcuts-and-deep-links.md)에 있습니다
- 마크다운 형식은 앱의 `src/markdown.ts`와 같습니다. 한쪽을 바꾸면 다른 쪽도 바꿉니다

스토어에 올리려면 `package.json`의 `author`를 Raycast 계정 이름으로 바꿔야 합니다. 지금 값(nobel6018)은 Raycast 계정이 아니라 `ray lint`에서 걸립니다.
