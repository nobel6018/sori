로컬에서 녹음을 받아 적고(whisper.cpp) 화자를 나누는(sherpa-onnx) 데스크탑 앱입니다.

## 설치
- macOS (Apple Silicon): `.dmg`
- Windows: `.msi` 또는 `-setup.exe`
- iPhone: AltStore에 소스 `https://github.com/nobel6018/sori/releases/latest/download/apps.json`을 추가해 설치합니다([설치 방법](https://github.com/nobel6018/sori/blob/main/docs/mobile.md))

코드 서명을 하지 않은 빌드입니다.
- macOS: dmg에서 `sori.app`을 응용 프로그램 폴더로 옮긴 뒤 터미널에서 아래 명령을 한 번 실행합니다. 브라우저로 받은 파일에 붙는 격리 표시를 지워, "손상되었기 때문에 열 수 없습니다" 경고 없이 열립니다

  ```sh
  xattr -cr /Applications/sori.app
  ```
- Windows: SmartScreen 경고가 뜨면 "추가 정보"를 눌러 "실행"을 누릅니다

0.9.3부터 Mac과 Windows 앱은 켤 때 새 버전을 알리고, "업데이트하고 다시 켜기"를 누르면 앱 안에서 설치합니다. 이때는 xattr 명령이 필요 없습니다. 녹음, 전사 같은 작업이 진행 중이면 설치하지 않습니다([업데이트](https://github.com/nobel6018/sori/blob/main/docs/update.md))

## 모델
앱을 처음 켜면 모델 설치 화면이 뜹니다. 버튼을 누르면 받아 적기 모델(1.6GB)과 화자 분리 모델을 받습니다.
