#!/usr/bin/env bash
# sori가 쓰는 모델을 내려받는다 (macOS, Linux)
# 보통은 앱이 처음 켜질 때 버튼으로 받는다. 이 스크립트는 터미널로 받고 싶을 때와 CI용이다
#
#   ./scripts/install-models.sh                     # Whisper large-v3-turbo (1.6GB) + 화자 분리 모델
#   ./scripts/install-models.sh --whisper large-v3  # Whisper large-v3 (3.1GB)
#   ./scripts/install-models.sh --whisper all       # 둘 다
#
# 저장소를 받지 않았다면:
#   curl -fsSL https://raw.githubusercontent.com/nobel6018/sori/main/scripts/install-models.sh | bash -s -- --whisper turbo
set -euo pipefail

WHISPER=turbo
case "$(uname -s)" in
  Darwin) DIR="$HOME/Library/Application Support/com.nobel6018.sori/models" ;;
  *) DIR="${XDG_DATA_HOME:-$HOME/.local/share}/com.nobel6018.sori/models" ;;
esac

while [ $# -gt 0 ]; do
  case "$1" in
    --whisper) WHISPER="$2"; shift 2 ;;
    --dir) DIR="$2"; shift 2 ;;
    -h|--help)
      # curl | bash로 실행하면 $0이 파일이 아니라서 도움말을 여기 직접 둔다
      echo "사용법: install-models.sh [--whisper turbo|large-v3|all|none] [--dir 폴더]"
      echo "  turbo     large-v3-turbo, 1.6GB, 빠름 (기본값)"
      echo "  large-v3  large-v3, 3.1GB, 조금 더 정확함. Mac에서 약 15% 느리고 GPU 없는 PC에서는 더 느림"
      exit 0 ;;
    *) echo "알 수 없는 옵션: $1 (--help 참고)" >&2; exit 1 ;;
  esac
done
case "$WHISPER" in turbo|large-v3|all|none) ;; *) echo "--whisper는 turbo, large-v3, all, none 중 하나입니다" >&2; exit 1 ;; esac

WHISPER_URL=https://huggingface.co/ggerganov/whisper.cpp/resolve/main
SHERPA_URL=https://github.com/k2-fsa/sherpa-onnx/releases/download
EMBEDDING=3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx
SEGMENTATION=pyannote-segmentation-3-0.onnx
# 압축본(tar.bz2)에는 예제와 int8 모델까지 들어 있어 이 파일 하나보다 크다
SEGMENTATION_URL=https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/main/model.onnx

mkdir -p "$DIR"
echo "모델 폴더: $DIR"

# 받는 중에는 .part로 두고 다 받으면 이름을 바꾼다. 끊겼다가 다시 실행하면 이어서 받는다
download() {
  local url="$1" dest="$2"
  if [ -s "$dest" ]; then
    echo "  이미 있음: $(basename "$dest")"
    return
  fi
  echo "  받는 중: $(basename "$dest")"
  curl -fL --retry 3 --progress-bar -C - -o "$dest.part" "$url"
  mv "$dest.part" "$dest"
}

if [ "$WHISPER" = turbo ] || [ "$WHISPER" = all ]; then
  download "$WHISPER_URL/ggml-large-v3-turbo.bin" "$DIR/ggml-large-v3-turbo.bin"
fi
if [ "$WHISPER" = large-v3 ] || [ "$WHISPER" = all ]; then
  download "$WHISPER_URL/ggml-large-v3.bin" "$DIR/ggml-large-v3.bin"
fi

download "$SHERPA_URL/speaker-recongition-models/$EMBEDDING" "$DIR/$EMBEDDING"

download "$SEGMENTATION_URL" "$DIR/$SEGMENTATION"

echo "끝났습니다. sori 설정에서 모델을 고를 수 있습니다."
