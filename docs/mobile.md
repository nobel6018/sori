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
| `mobile/src-tauri/Info.ios.plist` | 마이크 권한 설명, 백그라운드 오디오 |
| `design/logo-ios.svg`, `design/logo-ios-1024.png` | 앱 아이콘. iOS가 모서리를 깎으므로 데스크톱 로고의 여백, 둥근 모서리, 그림자를 뺐다. `pnpm tauri icon ../design/logo-ios-1024.png`로 만들고 알파 채널을 없앤다(투명한 곳은 검게 보인다) |
| `mobile/src-tauri/gen/apple/project.yml` | Xcode 프로젝트. 서명 없이 빌드하도록 `CODE_SIGNING_ALLOWED: NO` |
| `.github/workflows/release.yml` `ios` | 태그를 올리면 `sori-ios.ipa`를 만들어 Release에 붙인다 |

## 빌드

```sh
cd mobile
pnpm tauri ios build --target aarch64 --ci   # 빌드는 되고 내보내기(서명)에서 멈춘다
# DerivedData의 release-iphoneos/sori.app을 Payload/에 넣고 zip하면 .ipa
```

- Xcode가 기본 개발 도구로 잡혀 있지 않으면(`xcode-select -p`가 CommandLineTools) `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`를 줍니다
- 디버그 빌드는 화면을 개발 서버(localhost:1421)에서 읽어, 서버 없이 띄우면 검은 화면이 된다. 화면 확인은 릴리스 빌드로 한다
- `mobile/plugin/ios/Package.swift`의 swift-tools-version은 5.9 이상이어야 `.iOS(.v17)`을 쓸 수 있다
