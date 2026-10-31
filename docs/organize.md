# 태그로 모아 보기, AI 자동 분류, 녹음한 곳

녹음을 태그로 모아 보고, Mac이 받아 적은 녹음에 AI로 태그를 붙이고, 녹음을 시작한 곳을 남깁니다(0.8.0).

## 태그

폴더와 태그를 따로 두지 않고 태그 하나로 분류합니다. 태그 이름에 `/`를 넣으면 계층이 생겨 폴더처럼 모아 봅니다.

```
업무/영업/한빛로보틱스
업무/주간회의
개인/병원
```

- 한 녹음에 태그를 여러 개(최대 10개) 붙일 수 있습니다. "고객 미팅"이면서 "주간회의"인 녹음은 두 곳에 다 보입니다
- 상위 태그(`업무`)를 고르면 하위 태그(`업무/영업/...`)가 붙은 녹음도 보입니다
- 정리 규칙: 앞뒤 공백, `/` 앞뒤 공백, 빈 단계(`업무//영업`), 머리의 `#`을 없애고, 대소문자만 다른 태그는 하나로 칩니다(`sori_sync::tags::normalize_tags`, `src/tags.ts` `normalizeTags`)

| 화면 | Mac | iPhone |
|---|---|---|
| 목록 | 사이드바 위 태그 트리(`TagTree`). 누르면 걸러 보고, 고른 태그는 다음 실행에도 기억 | 목록 위 태그 칩 줄(`List.tsx` `TagBar`). 하위가 있는 태그를 누르면 그 안으로 들어감 |
| 녹음 | 제목 아래 태그 편집(`TagEditor`), 녹음한 곳 | 날짜 아래 태그 편집, 녹음한 곳 |
| 찾기 | 빠른 찾기(⌘K)가 제목, 태그, 장소를 초성으로도 찾음(`search.rs` `matches_with_initials`) | 태그 입력 자동완성이 초성으로 찾음 |

태그 입력은 Enter나 쉼표로 붙이고, 칩의 ×나 빈 칸에서 Backspace로 뺍니다. 자동완성은 화살표로 고른 경우에만 그 제안을 붙이고, 아니면 친 글자 그대로 새 태그가 됩니다.

## AI 자동 분류 (Mac)

받아 적기가 끝나면(자동 요약이 켜져 있으면 요약 뒤에) 요약과 같은 CLI(claude 또는 codex)로 태그를 1~3개 붙입니다. 설정 > "받아 적기가 끝나면 AI로 태그 붙이기"(기본 켜짐).

- 다른 녹음에 쓰인 태그와 녹음 수를 함께 넘겨 쓰던 태그를 먼저 고르게 합니다. 태그가 마구 늘지 않게 하려는 것입니다
- 넘기는 내용: 제목, 날짜, 장소, 메모, 요약(있으면), 전사 일부(요약이 있으면 앞 3천 자, 없으면 앞 8천 자와 뒤 4천 자)
- 사람이 태그를 고친 녹음(`tagsEditedAt`)은 건드리지 않습니다. AI가 도는 사이 고쳐도 저장 직전에 다시 확인해 지킵니다
- 한 번 분류한 녹음(`autoTaggedAt`)은 다시 하지 않습니다. 녹음 화면에 "AI" 표시가 붙고, 태그를 고치면 사라집니다
- iPhone에서 동기화로 새로 온 녹음도, 자동 처리가 켜져 있으면 받아 적고 분류합니다(`App.tsx` `onRecordingsSynced`). 업데이트 전부터 대기 중이던 녹음은 건드리지 않습니다
- 설정의 "태그 없는 녹음 AI로 분류"는 받아 적기가 끝났고 태그가 없는 녹음을 하나씩 차례로 분류합니다. 한꺼번에 돌리면 구독 사용량 한도에 금방 걸립니다
- 분류가 실패해도 알림을 띄우지 않고 개발자 콘솔에만 남깁니다. 분류는 덤이라 받아 적기와 요약을 방해하지 않습니다

## 녹음한 곳

녹음을 시작할 때 위치를 한 번 받아 주소로 바꿔 남깁니다. 녹음 내내 추적하지 않습니다. 기기마다 설정에서 끕니다(기본 켜짐).

| | iPhone | Mac |
|---|---|---|
| 위치 | GPS, Wi-Fi, 기지국(`requestLocation`, 최대 10초) | Wi-Fi(`CLLocationManager`, 최대 10초, 오차 100m 이하면 바로 끝) |
| 주소 | `CLGeocoder`(최대 8초) | `CLGeocoder`(최대 8초) |
| 권한 | "앱을 사용하는 동안". 처음 묻는 것은 앱 화면에서 녹음할 때만. 동작 버튼으로 시작하면 못 받을 수 있음 | 시스템 설정 > 개인정보 보호 및 보안 > 위치 서비스. 처음 녹음할 때 묻는 동안 시간이 지나면 다음 녹음부터 남음 |
| 설정 | 앱 설정 > "녹음할 때 위치 남기기"(`preferences.json`) | 설정 > "녹음할 때 위치 남기기"(`Settings.record_location`) |

- 위치를 못 받으면 조용히 건너뜁니다. 녹음은 그대로 됩니다
- Mac은 2분 넘은 위치를 쓰지 않습니다. 노트북을 옮긴 뒤 예전 장소가 찍히지 않게 하려는 것입니다
- Mac 앱은 개발자 인증서 없이 서명(ad-hoc)해 업데이트마다 서명이 바뀝니다. macOS가 위치 허락을 다시 물을 수 있습니다
- `CLGeocoder`는 macOS 26, iOS 26에서 MapKit으로 대체 표시됐지만, 그 API는 26부터라 최소 지원 버전(macOS 14.2, iOS 18)에서 쓸 수 없어 그대로 씁니다
- 장소를 누르면 Apple 지도로 엽니다. iPhone은 화면(WebView) 안에서 열면 앱 화면이 지도 페이지로 바뀌어 `UIApplication.open`으로 엽니다(`open_url`)

## meta.json 필드

| 필드 | 뜻 | 쓰는 쪽 |
|---|---|---|
| `tags` | 태그 목록 | 사람(두 기기), AI(Mac) |
| `tagsEditedAt` | 사람이 태그를 고친 시각. 있으면 AI가 덮어쓰지 않음 | 두 기기 |
| `autoTaggedAt` | AI가 분류한 시각 | Mac |
| `location` | `{ lat, lon, accuracy, place, capturedAt }` | 녹음한 기기 |

네 필드 모두 기기끼리 나눕니다(`crates/sori-sync/src/meta.rs` `SHARED`). 이 목록에 없는 필드는 클라우드에 올리지 않으므로, 0.7.x 앱이 녹음을 고치면 이 필드들이 빠집니다. Mac과 iPhone을 0.8.0 이상으로 함께 올립니다.

## 코드 위치

| 위치 | 역할 |
|---|---|
| `crates/sori-sync/src/tags.rs` | 태그 정리 규칙 |
| `crates/sori-sync/src/meta.rs` `SHARED` | 기기끼리 나누는 필드 |
| `src/tags.ts` | 태그 트리, 걸러 보기, 자동완성, 초성 찾기, 지도 링크(두 앱 화면이 함께 씀) |
| `src/components/TagEditor.tsx` | 태그 편집(두 앱이 함께 씀) |
| `src/components/TagTree.tsx`, `Sidebar.tsx` | Mac 사이드바 태그 트리 |
| `src-tauri/src/tagger.rs`, `lib.rs` `auto_tag_recording` | AI 자동 분류 |
| `src-tauri/src/location.rs`, `lib.rs` `record_location` | Mac 위치 |
| `src-tauri/src/search.rs` `matches_with_initials` | 초성 찾기 |
| `mobile/src/List.tsx` `TagBar`, `mobile/src/Detail.tsx` | iPhone 목록 칩 줄, 상세 |
| `mobile/plugin/ios/Sources/Location.swift`, `SoriNativePlugin.swift` `currentLocation`, `openUrl` | iPhone 위치, 지도 열기 |
| `mobile/src-tauri/src/lib.rs` `set_tags`, `record_location`, `Preferences` | iPhone 태그 저장, 위치, 설정 |
