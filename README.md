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
- 2초마다 WAV 헤더를 갱신해, 앱이 비정상 종료돼도 그때까지의 녹음은 재생됩니다
- macOS에서는 녹음하는 동안 App Nap과 잠자기를 막습니다. 앱이 뒤에 가려져 있어도 녹음 스레드가 느려지지 않게 하려는 것입니다
- 장치가 빠지면 녹음을 마무리하고 화면에 알립니다. 버퍼 오버런 같은 일시 경고로는 멈추지 않습니다

## 처리 방식

`src-tauri/src/engine/`

1. `audio.rs`: symphonia로 디코딩하고 16kHz 모노로 리샘플링
2. `diarize.rs`: 파일 전체에 화자 분리를 한 번 돌린다. 조각마다 돌리면 조각 사이에서 화자 번호가 어긋난다
3. `transcribe.rs`: 같은 화자 구간을 30초 이내 조각으로 묶어 조각마다 전사한다. 파일 전체를 한 번에 돌리면 긴 회의에서 반복 환각이 났다. 녹음 언어를 지정하면, 목록 밖 언어로 감지된 조각은 지정 언어로 다시 돌려 토큰 확률이 높은 결과를 쓴다

녹음과 결과는 앱 데이터 폴더(macOS: `~/Library/Application Support/com.nobel6018.sori/recordings/`)에 녹음마다 폴더 하나로 저장됩니다.
