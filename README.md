# sori

녹음 파일을 이 컴퓨터 안에서만 받아 적고(whisper.cpp) 화자를 나누는(sherpa-onnx) 데스크탑 앱입니다. Tauri 2 + React.

## 설치

[Releases](https://github.com/nobel6018/sori/releases/latest)에서 받습니다. macOS(Apple Silicon)는 `.dmg`, Windows는 `.msi` 또는 `-setup.exe`입니다.

코드 서명을 하지 않은 빌드라 macOS는 처음 한 번 격리 표시를 지워야 합니다. dmg에서 `sori.app`을 응용 프로그램 폴더로 옮긴 뒤 터미널에서 실행합니다.

```sh
xattr -cr /Applications/sori.app
```

지우지 않으면 "손상되었기 때문에 열 수 없습니다"라는 경고가 뜨고 앱이 열리지 않습니다. Windows는 SmartScreen 경고가 뜨면 "추가 정보"를 눌러 "실행"을 누릅니다.

## 모델 설치

모델은 설치 파일에 넣지 않고 따로 받습니다. 앱을 처음 켜면 모델이 없을 때 설치 화면이 뜨고, 버튼 하나로 받습니다. 설정에서도 Whisper 모델을 고르고 받을 수 있습니다(`src-tauri/src/download.rs`).

- 파일을 16MB 조각으로 나눠 연결 4개로 동시에 받습니다. 연결 하나당 속도를 묶는 서버나 지연이 큰 해외 경로에서 빨라집니다
- 조각마다 받은 바이트를 1초마다 기록해, 끊기거나 앱을 꺼도 다시 시작하면 받은 부분부터 이어 받습니다
- 다 받으면 크기를 확인한 뒤 파일 이름을 바꿉니다

터미널로 받고 싶으면 스크립트를 씁니다. 이미 받은 파일은 건너뛰고 끊기면 이어서 받습니다.

macOS, Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/nobel6018/sori/main/scripts/install-models.sh | bash -s -- --whisper turbo
```

Windows (PowerShell):

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/nobel6018/sori/main/scripts/install-models.ps1))) -Whisper turbo
```

`--whisper`(`-Whisper`)로 받아 적기 모델을 고릅니다. 받은 모델은 앱의 설정에서 고를 수 있습니다.

| 값 | 모델 | 크기 | 특징 |
|---|---|---|---|
| `turbo` (기본값) | ggml-large-v3-turbo | 1.6GB | 빠르고 정확도도 충분함 |
| `large-v3` | ggml-large-v3 | 3.1GB | 조금 더 정확함. Mac(Metal)에서 약 15% 느리고, GPU 없는 PC에서는 차이가 더 큼 |

3개 언어(한국어, 중국어, 영어) 5분 30초 회의로 잰 값입니다(M5 Pro, Metal).

| 모델 | 처리 시간 | 한국어 글자 오류율 | 중국어 | 영어 |
|---|---|---|---|---|
| large-v3-turbo | 151초 | 5.3% | 2.3% | 2.4% |
| large-v3 | 173초 | 4.9% | 1.6% | 2.4% |
| `all` | 둘 다 | 4.7GB | |
| `none` | 받지 않음 | | 화자 분리 모델만 받을 때 |

화자 분리 모델(pyannote segmentation 3.0, 3D-Speaker ERes2Net)은 항상 함께 받습니다. 받는 곳은 macOS `~/Library/Application Support/com.nobel6018.sori/models`, Windows `%APPDATA%\com.nobel6018.sori\models`입니다. 앱은 이 폴더를 먼저 보고, 예전 공용 위치(`~/whisper-models`, `~/sherpa-onnx/models`)도 찾습니다(`src-tauri/src/models.rs`).

## 녹음 뒤에 하는 일

- 자동 처리: 녹음을 멈추거나 파일을 가져오면 바로 받아 적습니다. 끝나면 요약까지 하게 할 수 있습니다(설정)
- 화자 목소리 기억: 화자 이름을 바꾸면 목소리를 기억하고, 다음 녹음에서 같은 사람에게 이름을 붙입니다. [docs/voices.md](docs/voices.md)
- 요약 템플릿: 회의록, 1:1 면담, 인터뷰, 강의, 할 일만. 한 녹음에 여러 요약을 남기고, 템플릿은 설정에서 고치고 더합니다
- 메모와 북마크: 녹음하며 메모를 쓰거나 북마크(⌘B, 뒤에 있으면 ⌥⇧⌘B)를 남기면 요약이 그 부분을 더 중요하게 다룹니다
- 질문하기: 녹음 하나(질문 탭) 또는 모든 녹음(⌘J)에 묻습니다. 답의 시각을 누르면 그 구간이 재생됩니다. [docs/ask.md](docs/ask.md)
- 내보내기: 마크다운, 텍스트, SRT, VTT, Word. [docs/export.md](docs/export.md)
- 회의 앱 감지: Zoom, Teams, Meet 등이 마이크를 쓰기 시작하면 녹음할지 묻습니다. [docs/meeting-detection.md](docs/meeting-detection.md)
- MCP 서버: Claude Code, Claude 데스크톱에서 sori 녹음을 찾고 읽습니다. [mcp/README.md](mcp/README.md)

## 단축키, 링크, Raycast

- 전역 단축키: 녹음 시작/멈춤 ⌥⇧⌘R(Windows Ctrl+Alt+Shift+R), 창 앞으로 ⌥⇧⌘S. 설정에서 바꿉니다
- 앱 안 단축키: ⌘K 빠른 찾기(미리 보기, 마크다운 복사), ⌘O 가져오기, ⌘↩ 처리, Space 재생 등. ⌘/로 전체 목록을 봅니다
- 녹음 파일 바로 열기: Finder에서 "다음으로 열기 > sori", 또는 Dock 아이콘에 끌어다 놓기
- `sori://` 링크: 녹음 토글, 특정 시각 열기, 가져오기, 검색. Raycast 확장(`raycast/`)이 이 링크를 씁니다

자세한 내용은 [docs/shortcuts-and-deep-links.md](docs/shortcuts-and-deep-links.md)와 [raycast/README.md](raycast/README.md)에 있습니다.

## 개발

```bash
pnpm install
pnpm tauri dev
```

엔진만 확인할 때는 앱 없이 돌릴 수 있습니다.

```bash
cd src-tauri
cargo run --release --example pipeline -- <오디오 파일> [화자 수|auto] [ko,zh]
```

## 플랫폼

- macOS: whisper.cpp를 Metal로 빌드합니다
- Windows: 기본은 CPU 빌드입니다. 빌드 PC에 SDK가 있으면 `pnpm tauri build -- --features vulkan` 또는 `--features cuda`

## 녹음

`src-tauri/src/recorder.rs`

- 설정에서 고른 마이크(기본값은 시스템 기본 마이크)로 장치 기본 샘플레이트의 16비트 모노 WAV를 녹음합니다. 1시간에 약 170MB(24kHz)에서 345MB(48kHz)입니다
- 설정에서 "시스템 소리도 함께 녹음"을 켜면 기본 출력 장치로 나가는 소리(화상 회의 상대 목소리 등)도 녹음합니다. macOS 14.6 이상은 Core Audio 프로세스 탭, Windows는 WASAPI 루프백을 씁니다(둘 다 cpal)
- 마이크(`mic.wav`)와 시스템 소리(`system.wav`)는 따로 녹음하고, 멈출 때 하나로 섞어 `audio.wav`를 만듭니다. 원래 트랙은 남겨 둡니다
- macOS에서 입력도 지원하는 출력 장치(일부 헤드셋, Microsoft Teams Audio 같은 가상 장치)는 cpal이 루프백 대신 그 장치의 마이크를 엽니다. 이때는 잘못 녹음하지 않고 오류로 알립니다. 장치 목록은 `cargo run --example devices`로 확인합니다
- 2초마다 WAV 헤더를 갱신해, 앱이 비정상 종료돼도 그때까지의 녹음은 재생됩니다
- 녹음하는 동안 잠자기를 막습니다. macOS는 App Nap까지 막아 앱이 뒤에 가려져 있어도 녹음 스레드가 느려지지 않게 하고, Windows는 `SetThreadExecutionState`로 유휴 잠자기를 막습니다
- 장치가 빠지면 녹음을 마무리하고 화면에 알립니다. 버퍼 오버런 같은 일시 경고로는 멈추지 않습니다

## 요약

`src-tauri/src/summarize.rs`

- 이 컴퓨터에 설치되고 로그인된 `claude` 또는 `codex` CLI를 하위 프로세스로 불러 요약합니다. 구독(Claude Pro/Max, ChatGPT Plus 등)에는 API 키가 없어서 HTTP API 대신 CLI를 씁니다
- 전사 텍스트만 보내고 녹음 파일은 보내지 않습니다
- 도구를 끄고(`claude --tools ""`, `codex --sandbox read-only`) 빈 임시 폴더에서 실행해 다른 파일에 손대지 않게 합니다
- Finder에서 연 macOS 앱은 터미널 PATH를 물려받지 못하므로, 로그인 셸에 PATH를 한 번 물어 CLI를 찾습니다
- 요약 지시문은 설정에서 바꿀 수 있습니다

## 처리 방식

`src-tauri/src/engine/`

1. `audio.rs`: symphonia로 디코딩하고 16kHz 모노로 리샘플링
2. `diarize.rs`: 파일 전체에 화자 분리를 한 번 돌린다. 조각마다 돌리면 조각 사이에서 화자 번호가 어긋난다. 앱에서는 별도 프로세스(`worker.rs`)로 돌려 취소하면 바로 멈춘다
3. `recluster.rs`: 구간마다 목소리 특징을 다시 뽑아 묶는다. 화자 수를 알면 그 수로, 모르면 유사도 0.45에서 멈춘다. [docs/speaker-count.md](docs/speaker-count.md)
4. `transcribe.rs`: 같은 화자 구간을 30초 이내 조각으로 묶어 조각마다 전사한다. 파일 전체를 한 번에 돌리면 긴 회의에서 반복 환각이 났다. 녹음 언어를 지정하면, 목록 밖 언어로 감지된 조각은 지정 언어로 다시 돌려 토큰 확률이 높은 결과를 쓴다
5. `origin.rs`: 마이크와 시스템 소리를 따로 녹음했으면, 화자마다 말한 구간에서 두 트랙의 에너지를 비교해 이쪽(마이크) 사람인지 원격 참석자인지 정한다. 마이크 쪽 화자가 한 명이면 이름을 "나"로 붙인다

녹음과 결과는 앱 데이터 폴더(macOS: `~/Library/Application Support/com.nobel6018.sori/recordings/`)에 녹음마다 폴더 하나로 저장됩니다.
