# sori

녹음 파일을 이 컴퓨터 안에서만 받아 적고(whisper.cpp) 화자를 나누는(sherpa-onnx) 데스크탑 앱입니다. Tauri 2 + React.

## 준비물

모델은 앱에 넣지 않고 경로로 가리킵니다. 기본 경로는 아래와 같고, 앱의 설정 화면에서 바꿀 수 있습니다.

| 모델 | 기본 경로 |
|---|---|
| Whisper (ggml) | `~/whisper-models/ggml-large-v3-turbo.bin` |
| 화자 구간 (pyannote segmentation 3.0) | `~/sherpa-onnx/models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx` |
| 화자 임베딩 (3D-Speaker) | `~/sherpa-onnx/models/3dspeaker_eres2net_base_zh-cn_16k.onnx` |

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
2. `diarize.rs`: 파일 전체에 화자 분리를 한 번 돌린다. 조각마다 돌리면 조각 사이에서 화자 번호가 어긋난다
3. `transcribe.rs`: 같은 화자 구간을 30초 이내 조각으로 묶어 조각마다 전사한다. 파일 전체를 한 번에 돌리면 긴 회의에서 반복 환각이 났다. 녹음 언어를 지정하면, 목록 밖 언어로 감지된 조각은 지정 언어로 다시 돌려 토큰 확률이 높은 결과를 쓴다
4. `origin.rs`: 마이크와 시스템 소리를 따로 녹음했으면, 화자마다 말한 구간에서 두 트랙의 에너지를 비교해 이쪽(마이크) 사람인지 원격 참석자인지 정한다. 마이크 쪽 화자가 한 명이면 이름을 "나"로 붙인다

녹음과 결과는 앱 데이터 폴더(macOS: `~/Library/Application Support/com.nobel6018.sori/recordings/`)에 녹음마다 폴더 하나로 저장됩니다.
