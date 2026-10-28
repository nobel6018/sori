# sori가 쓰는 모델을 내려받는다 (Windows)
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

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
Write-Host "모델 폴더: $Dir"

# 받는 중에는 .part로 두고 다 받으면 이름을 바꾼다. 끊겼다가 다시 실행하면 이어서 받는다
function Get-Model([string]$Url, [string]$Dest) {
    if ((Test-Path $Dest) -and (Get-Item $Dest).Length -gt 0) {
        Write-Host "  이미 있음: $(Split-Path $Dest -Leaf)"
        return
    }
    Write-Host "  받는 중: $(Split-Path $Dest -Leaf)"
    & $Curl -fL --retry 3 --progress-bar -C - -o "$Dest.part" $Url
    if ($LASTEXITCODE -ne 0) { throw "내려받기 실패: $Url" }
    Move-Item -Force "$Dest.part" $Dest
}

if ($Whisper -in 'turbo', 'all') { Get-Model "$WhisperUrl/ggml-large-v3-turbo.bin" (Join-Path $Dir 'ggml-large-v3-turbo.bin') }
if ($Whisper -in 'large-v3', 'all') { Get-Model "$WhisperUrl/ggml-large-v3.bin" (Join-Path $Dir 'ggml-large-v3.bin') }

Get-Model "$SherpaUrl/speaker-recongition-models/$Embedding" (Join-Path $Dir $Embedding)

$SegDest = Join-Path $Dir $Segmentation
if ((Test-Path $SegDest) -and (Get-Item $SegDest).Length -gt 0) {
    Write-Host "  이미 있음: $Segmentation"
} else {
    Write-Host "  받는 중: $Segmentation"
    $Tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
    New-Item -ItemType Directory -Path $Tmp | Out-Null
    try {
        & $Curl -fL --retry 3 --progress-bar -o (Join-Path $Tmp 'seg.tar.bz2') "$SherpaUrl/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
        if ($LASTEXITCODE -ne 0) { throw '화자 구간 모델을 받지 못했습니다' }
        # Windows 10 1803부터 들어 있는 tar.exe(bsdtar)가 bz2를 푼다
        tar -xjf (Join-Path $Tmp 'seg.tar.bz2') -C $Tmp
        if ($LASTEXITCODE -ne 0) { throw '압축을 풀지 못했습니다' }
        Move-Item -Force (Join-Path $Tmp 'sherpa-onnx-pyannote-segmentation-3-0/model.onnx') $SegDest
    } finally {
        Remove-Item -Recurse -Force $Tmp
    }
}

Write-Host '끝났습니다. sori를 다시 열면 설정에서 모델을 고를 수 있습니다.'
