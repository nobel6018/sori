로컬에서 녹음을 받아 적고(whisper.cpp) 화자를 나누는(sherpa-onnx) 데스크탑 앱입니다.

## 설치
- macOS (Apple Silicon): `.dmg`
- Windows: `.msi` 또는 `-setup.exe`
- iPhone: `sori-ios.ipa`를 AltStore로 설치합니다([설치 방법](https://github.com/nobel6018/sori/blob/main/docs/mobile.md))

코드 서명을 하지 않은 빌드입니다.
- macOS: dmg에서 `sori.app`을 응용 프로그램 폴더로 옮긴 뒤 터미널에서 아래 명령을 한 번 실행합니다. 브라우저로 받은 파일에 붙는 격리 표시를 지워, "손상되었기 때문에 열 수 없습니다" 경고 없이 열립니다

  ```sh
  xattr -cr /Applications/sori.app
  ```
- Windows: SmartScreen 경고가 뜨면 "추가 정보"를 눌러 "실행"을 누릅니다

## 모델
앱을 처음 켜면 모델 설치 화면이 뜹니다. 버튼을 누르면 받아 적기 모델(1.6GB)과 화자 분리 모델을 받습니다.
