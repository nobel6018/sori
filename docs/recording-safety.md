# iPhone 녹음 보존

iPhone 녹음은 어떤 경우에도 녹음한 소리를 잃지 않아야 합니다. 전화가 오거나, 앱이 강제 종료되거나, 파일 변환이 실패해도 그때까지 녹음한 부분은 남습니다.

## 왜 바꿨나 (0.15.0까지의 사고)

2026-11-18 #43 녹음이 Mac에서 재생되지 않았습니다. iPhone 로그로 확인한 순서는 이렇습니다.

| 시각 | 일어난 일 |
|---|---|
| 16:26 | 동작 버튼으로 녹음 시작 |
| 16:34:22 | 전화가 와서 iOS가 녹음기를 멈춤(audiomxd `Stop Now`, 일시정지가 아니라 정지) |
| 16:34:48 | 통화가 끝나 같은 `AVAudioRecorder`에 `record()`를 다시 부름. 정지된 녹음기의 `record()`는 같은 파일을 처음부터 새로 써서 앞의 8분이 사라짐 |
| 16:34:48 | 다시 시작한 녹음이 0.15초 만에 멈추고, 5초 뒤 iOS가 앱을 일시정지 |
| 16:48 | 멈춤. 녹음기가 이미 멈춰 있어 m4a 목차(moov)를 쓰지 못함. 길이 0초, 읽을 수 없는 파일 |

같은 날 17:09 전화 테스트(#46)로 같은 순서를 재현했습니다.

m4a는 녹음을 멈출 때 파일 끝에 목차를 써야 읽힙니다. 그래서 녹음 도중 앱이 죽어도 통째로 읽을 수 없게 됩니다.

## 지금 구조

녹음은 구간 WAV 파일로 쓰고, 멈춘 뒤 `audio.m4a` 하나로 합칩니다.

```
recordings/<id>/
  meta.json          status: recording → finishing → new
  parts/0001.wav     첫 구간 (16kHz 모노 16비트 PCM)
  parts/0002.wav     통화가 끝난 뒤 이어 녹음한 구간
  audio.m4a          합친 결과 (16kHz 모노 AAC 32kbps). 만들고 길이를 확인한 뒤 parts/를 지운다
```

| 상황 | 동작 |
|---|---|
| 전화가 울림(안 받음) | 녹음을 끊지 않는다(`setPrefersNoInterruptionsFromSystemAlerts`). 0.15.0까지는 울리는 동안 녹음이 멈췄다(#48, 15초) |
| 전화를 받음 | 지금 구간을 닫는다. 그때까지 녹음은 `parts/`에 정상 파일로 남는다. 통화 중에는 iOS가 마이크를 통화에 넘겨 어떤 앱도 녹음할 수 없다 |
| 통화가 끝남 | 새 녹음기로 새 구간 파일에 녹음한다. 앞 구간은 건드리지 않는다 |
| 통화 뒤 재개 실패 | 1.5초 뒤 실제로 녹음되는지 확인하고, 안 되면 "녹음이 멈췄습니다" 알림을 띄운다. 앱을 열면 새 구간으로 이어 녹음한다 |
| 녹음 중 앱이 죽음 | WAV는 앞에서부터 그대로 읽힌다. 다음 실행 때 헤더의 길이 값을 파일 크기에 맞춰 고치고 합친다 |
| 합치기 실패(디스크 부족 등) | 구간 파일을 그대로 두고 `finishError`를 남긴다. 다음 실행 때 다시 합친다 |
| 합칠 자리에 `audio.m4a`가 이미 있음 | 지우지 않고 `previous-<시각>.m4a`로 옮겨 둔다 |

녹음 화면은 멈춘 동안 "일시정지"를 보여 줍니다. 메모 시각은 구간을 이어 붙인 오디오의 시각입니다. 통화하던 시간은 오디오에 없으므로 시각도 건너뜁니다.

건너뛴 자리에는 메모가 자동으로 들어갑니다("통화로 녹음이 2분 10초 멈췄습니다"). 이유는 멈출 때 진행 중인 전화가 있는지(`CXCallObserver`)로 정하고, 없으면 "다른 앱 소리"로 적습니다. 1초 미만은 남기지 않습니다. 멈춘 채로 녹음을 끝내면 메모를 남기지 않습니다.

`recording`과 `finishing` 상태인 녹음은 동기화하지 않습니다. 덜 된 오디오가 클라우드에 먼저 올라가면 동기화가 오디오가 이미 있다고 보고 완성본을 다시 올리지 않기 때문입니다.

## 크기

| 파일 | 1시간 |
|---|---|
| 녹음 중 구간 WAV | 약 115MB (합친 뒤 지움) |
| 합친 audio.m4a | 약 15MB |

이전 형식(44.1kHz AAC 64kbps)은 1시간에 약 28MB였습니다. 전사 엔진(whisper)이 16kHz로 읽어 전사 품질은 같습니다. Mac은 다른 곳에서 가져온 파일도 48kbps로 줄여 올립니다.

## 코드 위치

| 위치 | 하는 일 |
|---|---|
| `mobile/plugin/ios/Sources/SegmentedRecorder.swift` `SegmentedRecorder` | 구간 녹음, 인터럽션 처리(`handleInterruption`), 재개와 확인(`resume`, `verifyRunning`), 멈춤 알림 |
| `mobile/plugin/ios/Sources/RecordingFiles.swift` `RecordingFiles.finalize` | 구간을 audio.m4a로 합치고 길이를 확인한 뒤 구간을 지운다 |
| `mobile/plugin/ios/Sources/RecordingFiles.swift` `WavFile` | 앱이 죽어 길이가 0으로 남은 WAV 헤더 고치기(`repair`) |
| `mobile/plugin/ios/Sources/SoriNativePlugin.swift` `finalizeRecording` | 합치기를 메인 스레드 밖에서, 백그라운드 시간을 받아 실행 |
| `mobile/plugin/ios/Sources/SegmentedRecorder.swift` `markPaused`, `recordGap` | 멈춘 시각과 이유를 기록하고, 이어지면 빠진 구간으로 남긴다 |
| `mobile/src-tauri/src/lib.rs` `gap_note_text` | 빠진 구간을 메모 문구로 만든다 |
| `mobile/src-tauri/src/lib.rs` `end_recording`, `finish_in_background` | 멈추면 `finishing`으로 두고 합친 뒤 `new`로 바꿔 올린다. 앱을 켤 때 `recording`, `finishing`으로 남은 녹음을 다시 합친다 |
| `mobile/src-tauri/src/sync.rs` `MobileLocal::is_busy` | 녹음 중, 합치는 중인 녹음은 동기화하지 않는다 |

## 테스트

```sh
scripts/test-recording-files.sh   # Mac에서 합치기와 복구를 검사한다(Xcode 필요)
```

정상 종료, 헤더 길이가 0인 파일(앱 종료), 헤더가 일부만 적힌 파일, 빈 구간, 실패 뒤 다시 합치기, 기존 파일 보존을 검사합니다(`mobile/plugin/ios/Tests/RecordingFilesTests.swift`). 합친 형식을 Mac 전사 엔진이 읽는지는 `src-tauri/src/sync.rs`의 `iphone_녹음_형식_16khz_aac를_전사_엔진이_읽는다`가 봅니다.

전화 인터럽션은 iPhone에서 직접 확인합니다. 동작 버튼으로 녹음을 시작하고 두 가지를 해 봅니다.

1. 전화가 울리게만 두고 받지 않는다. 녹음이 끊기지 않아 오디오 길이가 녹음 시간 전체와 같아야 하고, 메모가 생기지 않는다
2. 전화를 받아 통화하고 끊는다. 오디오 길이가 통화 시간을 뺀 녹음 시간과 같고, 끊긴 자리에 "통화로 녹음이 N초 멈췄습니다" 메모가 있어야 한다 로그는 iPhone을 연결한 Mac에서 `uvx pymobiledevice3 syslog live -ei 'sori.*(Interruption|녹음)'`으로 봅니다.
