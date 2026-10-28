# sori가 쓰는 모델을 내려받는다 (Windows)
# 보통은 앱이 처음 켜질 때 버튼으로 받는다. 이 스크립트는 터미널로 받고 싶을 때와 CI용이다
#
#   .\scripts\install-models.ps1                    # Whisper large-v3-turbo (1.6GB) + 화자 분리 모델
#   .\scripts\install-models.ps1 -Whisper large-v3  # Whisper large-v3 (3.1GB)
#   .\scripts\install-models.ps1 -Whisper all       # 둘 다
#
# 저장소를 받지 않았다면 PowerShell에서:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/nobel6018/sori/main/scripts/install-models.ps1))) -Whisper turbo
#
# 이 파일은 UTF-8 BOM으로 저장한다. BOM이 없으면 Windows PowerShell 5.1이 한글 문구를 깨뜨려 읽는다
param(
    [ValidateSet('turbo', 'large-v3', 'all', 'none')]
    [string]$Whisper = 'turbo',
    [string]$Dir = $(if ($env:APPDATA) { Join-Path $env:APPDATA 'com.nobel6018.sori\models' } else { Join-Path $HOME '.local/share/com.nobel6018.sori/models' })
)
$ErrorActionPreference = 'Stop'
# PowerShell의 curl은 Invoke-WebRequest 별칭이라 진짜 curl.exe를 지정한다(Windows 10 1803부터 기본 포함)
$Curl = if ($env:OS -eq 'Windows_NT') { 'curl.exe' } else { 'curl' }

$WhisperUrl = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main'
$SherpaUrl = 'https://github.com/k2-fsa/sherpa-onnx/releases/download'
$Embedding = '3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx'
$Segmentation = 'pyannote-segmentation-3-0.onnx'
# 압축본(tar.bz2)에는 예제와 int8 모델까지 들어 있어 이 파일 하나보다 크다
$SegmentationUrl = 'https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/main/model.onnx'

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
Write-Host "모델 폴더: $Dir"

# curl을 Start-Process로 실행한다. Windows PowerShell 5.1은 stderr가 리다이렉트된 환경(CI, ISE 등)에서
# 네이티브 프로그램의 stderr 줄을 오류 레코드로 바꾸는데, curl 진행 막대가 stderr라 'Stop'에서 바로 멈춘다.
# Start-Process는 출력을 파이프라인을 거치지 않고 콘솔로 보낸다. 성공 여부는 종료 코드로 본다
function Invoke-Curl([string[]]$Arguments) {
    # 공백이 있는 경로(C:\Users\Lee Young\...)가 깨지지 않게 인자를 직접 따옴표로 감싼다
    $line = ($Arguments | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }) -join ' '
    $p = Start-Process -FilePath $Curl -ArgumentList $line -NoNewWindow -Wait -PassThru
    return $p.ExitCode
}

# 받는 중에는 .part로 두고 다 받으면 이름을 바꾼다. 끊겼다가 다시 실행하면 이어서 받는다
function Get-Model([string]$Url, [string]$Dest) {
    if ((Test-Path $Dest) -and (Get-Item $Dest).Length -gt 0) {
        Write-Host "  이미 있음: $(Split-Path $Dest -Leaf)"
        return
    }
    Write-Host "  받는 중: $(Split-Path $Dest -Leaf)"
    $code = Invoke-Curl @('-fL', '--retry', '3', '--progress-bar', '-C', '-', '-o', "$Dest.part", $Url)
    if ($code -ne 0) { throw "내려받기 실패(curl 종료 코드 $code): $Url" }
    Move-Item -Force "$Dest.part" $Dest
}

if ($Whisper -in 'turbo', 'all') { Get-Model "$WhisperUrl/ggml-large-v3-turbo.bin" (Join-Path $Dir 'ggml-large-v3-turbo.bin') }
if ($Whisper -in 'large-v3', 'all') { Get-Model "$WhisperUrl/ggml-large-v3.bin" (Join-Path $Dir 'ggml-large-v3.bin') }

Get-Model "$SherpaUrl/speaker-recongition-models/$Embedding" (Join-Path $Dir $Embedding)

Get-Model $SegmentationUrl (Join-Path $Dir $Segmentation)

Write-Host '끝났습니다. sori 설정에서 모델을 고를 수 있습니다.'
