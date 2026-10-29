# 회의 앱 감지

회의 앱이 마이크를 쓰기 시작하면 화면 오른쪽 위에 "녹음할까요?" 창을 띄웁니다(`src-tauri/src/meeting.rs`, `src/components/MeetingPrompt.tsx`). 회의 앱을 보는 중이라 이 창은 초점을 가져가지 않고, 30초 동안 답하지 않으면 닫힙니다. 설정에서 끌 수 있습니다.

## 마이크를 쓰는 앱 찾기

3초마다 확인합니다(`lib.rs` `watch_meetings`).

- macOS: Core Audio의 프로세스 목록(`kAudioHardwarePropertyProcessObjectList`, macOS 14.0 이상)에서 마이크 입력 중(`kAudioProcessPropertyIsRunningInput`)인 프로세스의 번들 id를 읽습니다. 권한이 필요 없습니다. `cargo run --example meeting_check`로 지금 오디오를 쓰는 프로세스를 볼 수 있습니다
- Windows: 설정 앱의 "마이크를 사용 중인 앱" 기록(`HKCU\...\CapabilityAccessManager\ConsentStore\microphone`)에서 시작 시각은 있고 끝난 시각이 0인 앱을 찾습니다. 일반 앱은 실행 파일 경로, 스토어 앱(새 Teams)은 패키지 이름입니다

## 알아보는 앱

Zoom, Microsoft Teams, Webex, Slack 허들, Discord, FaceTime, 카카오톡 통화, 그리고 브라우저(Chrome, Safari, Arc, Edge, Firefox, Whale)입니다(`meeting::KNOWN`). 브라우저는 Meet인지 다른 사이트인지 알 수 없어 "Chrome (Google Meet 등)"으로 묻습니다. 목록에 없는 앱은 묻지 않습니다.

## 언제 묻나

| 상황 | 동작 |
|---|---|
| 회의 앱이 새로 마이크를 쓰기 시작, sori는 녹음 중이 아님 | 녹음할지 묻는다 |
| 계속 마이크를 쓰는 중 | 다시 묻지 않는다. 마이크를 놓았다가 다시 쓰면 다시 묻는다 |
| 이 질문으로 시작한 녹음인데, 그 앱이 마이크를 15초 넘게 놓음 | 멈출지 묻는다 |
| 직접 시작한 녹음 | 멈추라고 묻지 않는다 |

잠깐 음소거해도 마이크를 놓는 앱이 있어서 15초를 기다립니다(`meeting::STOP_GRACE`). 판단 규칙은 `meeting::Detector`에 시스템 호출과 떼어 두고 테스트로 확인합니다.
