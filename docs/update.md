# 데스크톱 앱 업데이트

Mac과 Windows의 sori는 앱 안에서 새 버전을 받아 설치합니다. 0.9.3부터 들어 있어서, 0.9.2 이하에서는 한 번만 Release에서 직접 받아 설치합니다.
iPhone 앱은 AltStore 소스로 업데이트합니다(docs/mobile.md).

## 동작

1. 앱을 켜면 `releases/latest/download/latest.json`을 읽어 새 버전이 있는지 확인합니다
   손으로 확인하려면 macOS 메뉴 sori > "업데이트 확인하기…"(시스템 첫 언어가 한국어가 아니면 "Check for Updates…", `system_prefers_korean`)(`lib.rs` `add_update_menu`, 화면에 `check-update` 이벤트)나 설정 맨 위 "업데이트 확인하기"를 누릅니다. 새 버전이 없으면 "최신 버전(x.y.z)입니다"를 보여 줍니다
2. 있으면 화면 위에 "새 버전 x.y.z" 알림과 "업데이트하고 다시 켜기" 버튼을 띄웁니다. 저절로 설치하거나 다시 켜지 않습니다
3. 버튼을 누르면 진행 중인 작업을 확인합니다. 하나라도 있으면 설치하지 않고 "진행 중인 작업 [녹음 #12 회의, 전사 #11 ...]을 완료하거나 종료한 뒤 다시 눌러 주세요"를 보여 줍니다
4. 작업이 없으면 내려받고, 설치 직전에 한 번 더 확인한 뒤 설치하고 다시 켭니다. 받는 사이에 녹음을 시작했으면 설치하지 않고 3번 문구를 보여 줍니다. 받은 파일은 두었다가 다음에 누를 때 다시 받지 않습니다

진행 중인 작업으로 보는 것(`src-tauri/src/lib.rs` `busy_tasks`):

| 작업 | 상태 |
|---|---|
| 녹음 | `AppState.recorder` |
| 전사, 화자 분리 | `AppState.cancels` |
| 요약 | `AppState.summaries` |
| AI 정리(제목, 태그, 할 일) | `tagger::running` |
| 모델 내려받기 | `AppState.model_install` |
| 동기화 | `Syncer.status().running` |

"모든 녹음에 묻기"는 넣지 않았습니다. 끊겨도 다시 물으면 되고, 저장하는 것이 없습니다.

다시 켜면 화면에만 있던 값(저장하지 않은 설정 창 수정, 검색어)은 사라집니다. 녹음, 전사, 요약, 메모는 파일로 저장돼 있어 그대로 남습니다.

## xattr이 필요 없는 이유

브라우저로 받은 dmg에는 macOS가 격리 표시(`com.apple.quarantine`)를 붙이고, 서명하지 않은 앱은 이 표시가 있으면 "손상되었기 때문에 열 수 없습니다"로 막힙니다. 그래서 처음 설치할 때는 `xattr -cr`로 지웁니다.
앱 안의 업데이트는 sori가 직접 받아 풀기 때문에 격리 표시가 붙지 않습니다.

## 서명

업데이트 파일은 Apple 서명이 아니라 Tauri updater 서명(minisign)으로 확인합니다. 무료이고 Apple 개발자 계정이 필요 없습니다.

| 항목 | 위치 |
|---|---|
| 공개 키 | `src-tauri/tauri.conf.json` `plugins.updater.pubkey`(앱에 박힌다) |
| 비밀 키 | 이 Mac의 `~/.tauri/sori-updater.key`(비밀번호 없음), GitHub 저장소 secret `TAURI_SIGNING_PRIVATE_KEY` |

비밀 키를 잃으면 이미 설치된 앱에 업데이트를 보낼 수 없습니다(새 키로 만든 앱을 다시 손으로 설치해야 한다). 키를 따로 백업해 둡니다.

`createUpdaterArtifacts`가 켜져 있어 `pnpm tauri build`는 비밀 키가 없으면 실패합니다. 로컬에서 설치 파일을 만들 때는 아래처럼 줍니다.

```sh
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/sori-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" pnpm tauri build
```

## 릴리스

`v*` 태그를 올리면 release.yml이 아래를 Release에 붙입니다.

| 파일 | 쓰임 |
|---|---|
| `sori_x.y.z_aarch64.app.tar.gz`, `.sig` | macOS 업데이트 |
| `sori_x.y.z_x64-setup.exe`, `.sig` | Windows 업데이트(설치 파일과 같은 파일) |
| `latest.json` | 위 두 파일의 주소와 서명. `updater` 작업이 두 OS 빌드가 끝난 뒤 만든다(`.github/scripts/updater-manifest.sh`) |

한 OS 빌드가 실패하면 `updater` 작업이 돌지 않아 `latest.json`이 없습니다. 그러면 앱은 GitHub API로 새 버전만 확인해 "내려받기"(Release 페이지) 버튼을 보여 줍니다(`src/updateCheck.ts`).

## 코드 위치

| 위치 | 내용 |
|---|---|
| `src/useAppUpdate.ts` | 확인, 진행 중인 작업 확인, 받기, 설치, 다시 켜기 |
| `src/components/UpdateBanner.tsx` | 알림 문구와 버튼. 화면 위 알림과 설정의 업데이트 줄이 함께 쓴다 |
| `src/components/SettingsDialog.tsx` | 설정 맨 위 버전과 "업데이트 확인하기" |
| `src-tauri/src/lib.rs` `add_update_menu` | macOS 앱 메뉴 항목 |
| `src-tauri/src/lib.rs` `busy_tasks` | 진행 중인 작업 목록 |
| `src-tauri/capabilities/default.json` | `updater:default`, `process:allow-restart` |
