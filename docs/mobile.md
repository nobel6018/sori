# iPhone 앱

iPhone에서 녹음하고 메모를 남기고, Mac의 sori가 만든 전사와 요약을 읽는 앱입니다. 받아 적기와 요약은 하지 않습니다(모델이 크고 휴대폰에서 느림).
Mac과는 iCloud Drive의 `sori` 폴더로 주고받습니다(docs/sync.md).

## 설치 (AltStore)

App Store에 올리지 않고 AltStore로 본인 iPhone에만 설치합니다. 무료 Apple ID로 됩니다.

1. Mac에 [AltServer](https://altstore.io)를 설치하고, iPhone을 케이블로 연결해 AltStore를 설치합니다
2. iPhone 설정 > 일반 > VPN 및 기기 관리에서 본인 Apple ID 개발자를 신뢰합니다. iOS 16 이상은 설정 > 개인정보 보호 및 보안 > 개발자 모드도 켭니다
3. [Releases](https://github.com/nobel6018/sori/releases/latest)에서 `sori-ios.ipa`를 iPhone으로 받습니다(Safari로 받으면 파일 앱의 다운로드 폴더에 들어갑니다)
4. AltStore > My Apps > 왼쪽 위 `+` > `sori-ios.ipa`를 고릅니다
5. sori를 열고 설정 > iCloud Drive를 눌러 `iCloud Drive/sori` 폴더를 고릅니다. Mac의 sori 설정에서 동기화를 iCloud Drive로 켜 두어야 이 폴더가 있습니다

iOS 18 이상이 필요합니다. 녹음 중 화면(Live Activity)을 그리는 위젯 확장이 앱 안에 함께 들어 있어, 무료 Apple ID의 App ID(7일에 10개)를 2개 씁니다.

무료 Apple ID 서명은 7일 뒤 만료됩니다. Mac에서 AltServer가 켜져 있고 iPhone과 같은 Wi-Fi에 있으면 AltStore가 자동으로 다시 서명합니다.
만료돼도 iCloud에 올라간 녹음은 남지만, 올리기 전에 지운 앱 안의 녹음은 사라집니다. 앱은 녹음을 멈추면 바로 올립니다.

## 동작

| 기능 | 방법 |
|---|---|
| 녹음 | AVAudioRecorder, AAC 모노 64kbps(1시간 약 28MB). 화면을 끄거나 다른 앱으로 가도 이어진다(백그라운드 오디오). 전화가 오면 멈췄다가 끝나면 이어서 녹음한다 |
| 메모, 북마크 | 녹음 중에는 녹음한 시각에, 녹음 뒤에는 재생 위치에 남긴다. 기기별 메모 파일로 Mac과 합친다 |
| 전사, 요약 | Mac이 만든 transcript.json, summaries.json을 읽기만 한다 |
| 오디오 | 다른 기기에서 온 녹음은 "오디오 받기"를 눌러야 받는다 |
| 동기화 | 앱을 열 때, 녹음을 멈출 때, 메모를 고칠 때, 앱이 앞에 있는 동안 20초마다. 목록 위 "↻ 동기화"는 기다리지 않고 바로 맞춘다 |

## 동작 버튼, 단축어로 바로 녹음

앱을 열지 않고 녹음을 시작하고 멈춥니다. 단축어를 따로 만들 필요는 없습니다.

1. 설정 > 동작 버튼 > 단축어 > sori > "녹음 시작/멈춤"을 고릅니다
2. 동작 버튼을 누르면 녹음이 시작되고 잠금 화면과 Dynamic Island에 경과 시간이 뜹니다
3. 동작 버튼을 다시 누르거나 Live Activity의 빨간 멈춤 버튼을 누르면 멈추고 iCloud Drive로 올립니다

| 동작 | 쓰는 곳 |
|---|---|
| 녹음 시작/멈춤 (`ToggleRecordingIntent`) | 동작 버튼, Siri("sori 녹음") |
| 녹음 시작 (`StartRecordingIntent`) | 단축어 자동화, Siri("sori 녹음 시작") |
| 녹음 멈추기 (`StopRecordingIntent`) | Live Activity 멈춤 버튼, Siri("sori 녹음 멈춰") |

설정 > 앱 > sori에서 실시간 현황(Live Activity)이 꺼져 있으면 시작하지 않고 그렇다고 알립니다. 앱 밖에서 녹음하는 `AudioRecordingIntent`는 녹음하는 동안 Live Activity를 띄워야 하고, 못 띄우면 시스템이 녹음을 끊기 때문입니다.
앱 안의 녹음 버튼으로 시작해도 같은 Live Activity가 뜹니다.

### 구조

```
동작 버튼 ─> ToggleRecordingIntent ─> RecordingControl ─(C 함수)─> intents.rs ─> begin_recording / end_recording (lib.rs)
                                                                                    ├─> SoriNativePlugin (AVAudioRecorder)
                                                                                    ├─> Live Activity 훅 (RecordingControl.swift)
                                                                                    └─> recorder-changed 이벤트 ─> 화면(App.tsx)
```

- 인텐트는 Tauri 플러그인이 아니라 Xcode 앱 타깃(`gen/apple/Sources/sori-mobile/`)에 둡니다. 플러그인 Swift는 swift-rs가 `libapp.a`로 컴파일해서, Xcode가 단축어 앱이 읽는 메타데이터(`Metadata.appintents`)를 뽑지 못합니다
- 녹음 상태는 화면 버튼과 똑같이 Rust의 `AppState.recording` 하나로 다룹니다. Swift는 `sori_intent_*` C 함수를 메인 스레드 밖에서 부릅니다. Rust가 메인 스레드의 Swift 녹음기를 기다리기 때문입니다
- Rust가 Swift의 Live Activity 함수를 부를 때는 링크가 아니라 함수 포인터를 씁니다(`sori_set_activity_hooks`, `main.mm`에서 등록). 크레이트가 cdylib도 만들어서 Swift 심볼을 직접 링크하면 미해결 심볼로 빌드가 깨집니다
- Live Activity는 녹음(오디오 세션)보다 먼저 띄웁니다. 앱 밖에서 시작하면 iOS가 Live Activity가 떠 있을 때만 백그라운드에서 오디오 세션을 켜 줍니다. 순서가 반대였던 0.7.0은 "Session activation failed"로 실패했습니다
- 단축어, 동작 버튼은 인텐트의 `perform()` 안에서 Live Activity를 직접 띄웁니다(`RecordingControl.start`). 0.7.1은 Rust를 거쳐 다른 스레드에서 띄우다 실패했습니다. 화면 버튼으로 시작할 때만 Rust 훅(`begin_recording`)이 띄웁니다
- 시작 인텐트는 `AudioRecordingIntent`와 `LiveActivityIntent`를 함께 선언합니다. `LiveActivityIntent`가 있어야 단축어 실행기가 Live Activity 시작 허가(activity request assertion)를 붙여 줍니다. 없던 0.7.2는 `visibility` 오류로 실패했습니다(기기 로그: `Requester is not targeted by an activity request assertion`)
- 백그라운드에서 시작하면 오디오 세션을 `.mixWithOthers`로 켭니다(`SoriNativePlugin.startRecording`). 백그라운드 앱은 다른 앱의 소리를 멈출 수 없어, 섞지 않는 세션은 "Session activation failed"(-12985)로 실패했습니다. 그래서 음악이 나오는 중에 동작 버튼으로 녹음하면 음악이 멈추지 않고 함께 녹음됩니다
- 섞을지는 앱 상태(`applicationState`)가 아니라 단축어에서 왔는지(`begin_recording`의 `from_intent`)로 정합니다. 정지에서 막 깨어난 앱은 `applicationState`가 active로 나와 섞지 않는 세션으로 실패했습니다(0.7.4, `!int` 560557684)
- 백그라운드에서 시작하면 audiomxd가 가끔 재생 권한 검사(`IsAllowedToStartPlaying`: "does not have assertions to start mixable playback")로 `record()`를 거절합니다. 같은 조건에서 성공할 때는 녹음 검사(`IsAllowedToStartRecording`)를 통과합니다. 원인은 아직 모릅니다(백그라운드 시작 6번에 1번꼴). 같은 세션으로 다시 하는 것은 소용없어, 세션을 껐다 켜며 3번까지 시도하고 몇 번째에 됐는지 기기 로그(`sori: ...번째 시도`)에 남깁니다
- 시작이 실패하면 오디오 세션을 끕니다. 켜진 채 남으면 다음 시작에서 카테고리를 바꾸는 것을 iOS가 다른 앱 소리를 끊으려는 시도로 보고 `!int`로 거절해, 실패 한 번이 뒤의 시도를 줄줄이 실패시켰습니다(0.7.5)
- 동작 버튼으로 녹음을 멈추면 Dynamic Island에 완료 표시(동그라미 체크)가 몇 초 뜹니다. 그동안 다시 누르면 iOS가 새로 실행하지 않고 끝난 실행기를 다시 써서 아무 일도 일어나지 않습니다(SpringBoard: `found running executor`). 앱에서 바꿀 수 없습니다
- 띄우지 못하면 iOS가 준 오류 이름(`visibility`, `unsupportedTarget` 등)을 알림 문구에 그대로 넣습니다
- 앱이 꺼져 있으면 iOS가 앱을 백그라운드로 띄웁니다. Rust는 Tauri setup이 끝날 때까지 최대 10초 기다린 뒤 녹음을 시작합니다
- `StopRecordingIntent`는 앱과 위젯 확장이 함께 컴파일합니다(`gen/apple/Shared/`). 버튼은 위젯이 그리지만 실행은 앱 프로세스에서 하므로, 위젯 쪽에서는 `WIDGET_EXTENSION` 조건으로 Rust 호출을 뺍니다

## iCloud Drive 접근

무료 Apple ID로 서명한 앱은 iCloud 권한(CloudKit, 앱 전용 iCloud 폴더)을 쓸 수 없습니다. 그래서 파일 선택 창으로 폴더를 한 번 고르고,
그 접근 권한(security-scoped bookmark)을 저장해 다음 실행에도 씁니다. 받지 않은 iCloud 파일은 읽기 전에 NSFileCoordinator로 받아 옵니다.

## 코드 위치

| 위치 | 역할 |
|---|---|
| `mobile/src/` | 화면(React). 목록 `List.tsx`, 녹음 `Recorder.tsx`, 상세 `Detail.tsx`, 설정 `Settings.tsx`. 전사 묶기와 시간 표시는 데스크톱 코드(`src/markdown.ts`, `src/api.ts`)를 같이 쓴다 |
| `mobile/src-tauri/src/lib.rs` | 화면이 부르는 명령: 목록, 상세, 제목, 메모, 녹음 시작과 멈춤, 동기화 |
| `mobile/src-tauri/src/store.rs` `Store` | 기기 저장소(데스크톱과 같은 폴더 구조, meta.json은 JSON 값 그대로) |
| `mobile/src-tauri/src/sync.rs` `MobileLocal`, `remote` | 동기화 엔진(crates/sori-sync) 연결, 폴더 권한 열기 |
| `mobile/plugin/ios/Sources/SoriNativePlugin.swift` | 녹음, 폴더 고르기와 권한 저장, iCloud 파일 받기 |
| `mobile/src-tauri/src/intents.rs` | 단축어 인텐트가 부르는 C 함수, Live Activity 훅 보관 |
| `mobile/src-tauri/gen/apple/Sources/sori-mobile/RecordingIntents.swift` | 녹음 시작, 시작/멈춤 인텐트와 단축어 목록(`SoriShortcuts`) |
| `mobile/src-tauri/gen/apple/Sources/sori-mobile/RecordingControl.swift` | Rust 호출, Live Activity 띄우고 내리기 |
| `mobile/src-tauri/gen/apple/Shared/` | 앱과 위젯이 함께 쓰는 Live Activity 데이터, 멈춤 인텐트 |
| `mobile/src-tauri/gen/apple/Widgets/SoriWidgets.swift` | Live Activity 화면(잠금 화면, Dynamic Island) |
| `mobile/src-tauri/Info.ios.plist` | 마이크 권한 설명, 백그라운드 오디오, Live Activity 사용 |
| `design/logo-ios.svg`, `design/logo-ios-1024.png` | 앱 아이콘. iOS가 모서리를 깎으므로 데스크톱 로고의 여백, 둥근 모서리, 그림자를 뺐다. `pnpm tauri icon ../design/logo-ios-1024.png`로 만들고 알파 채널을 없앤다(투명한 곳은 검게 보인다) |
| `mobile/src-tauri/gen/apple/project.yml` | Xcode 프로젝트. 서명 없이 빌드하도록 `CODE_SIGNING_ALLOWED: NO`. 위젯 확장 타깃 `sori-widgets`와 버전 맞추기 스크립트 |
| `.github/workflows/release.yml` `ios` | 태그를 올리면 `sori-ios.ipa`를 만들어 Release에 붙인다 |

## 빌드

```sh
cd mobile
pnpm tauri ios build --target aarch64 --ci   # 빌드는 되고 내보내기(서명)에서 멈춘다
# DerivedData의 release-iphoneos/sori.app을 Payload/에 넣고 zip하면 .ipa
```

- Xcode가 기본 개발 도구로 잡혀 있지 않으면(`xcode-select -p`가 CommandLineTools) `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`를 줍니다
- 디버그 빌드는 화면을 개발 서버(localhost:1421)에서 읽어, 서버 없이 띄우면 검은 화면이 된다. 화면 확인은 릴리스 빌드로 한다
- `project.yml`을 고친 뒤 `xcodegen generate`를 돌리면 `sori-mobile_iOS/Info.plist`가 project.yml 값으로 덮여 마이크 권한, 버전이 빠집니다. 빌드 때 Tauri가 `Info.ios.plist`를 다시 합치지만, 저장소의 파일은 `git checkout`으로 되돌려 둡니다
- `mobile/plugin/ios/Package.swift`의 swift-tools-version은 5.9 이상이어야 `.iOS(.v17)`을 쓸 수 있다
