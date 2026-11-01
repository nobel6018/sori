#!/usr/bin/env bash
# 데스크톱 앱 자동 업데이트(tauri-plugin-updater)가 읽는 latest.json을 만든다. release.yml updater 작업이 부른다.
# 앱은 tauri.conf.json plugins.updater.endpoints의 releases/latest/download/latest.json을 읽고,
# 여기 적힌 파일을 받아 .sig 서명을 앱에 박힌 공개 키로 확인한 뒤 설치한다(docs/update.md)
set -euo pipefail
: "${TAG:?}" "${GITHUB_REPOSITORY:?}"
version=${TAG#v}
base="https://github.com/$GITHUB_REPOSITORY/releases/download/$TAG"
dir=$(mktemp -d)
gh release download "$TAG" --repo "$GITHUB_REPOSITORY" -D "$dir" -p '*.sig'
ls "$dir"

mac="sori_${version}_aarch64.app.tar.gz"
win="sori_${version}_x64-setup.exe"
platforms='{}'
add() {
  local key=$1 file=$2
  if [ ! -f "$dir/$file.sig" ]; then echo "$file.sig가 없어 $key는 빼고 만듭니다"; return; fi
  platforms=$(jq --arg k "$key" --arg sig "$(cat "$dir/$file.sig")" --arg url "$base/$file" \
    '. + {($k): {signature: $sig, url: $url}}' <<<"$platforms")
}
add darwin-aarch64 "$mac"
add windows-x86_64 "$win"
if [ "$platforms" = '{}' ]; then echo "업데이트 파일이 하나도 없습니다"; exit 1; fi

jq -n --arg v "$version" --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg notes "https://github.com/$GITHUB_REPOSITORY/releases/tag/$TAG" --argjson p "$platforms" \
  '{version: $v, notes: $notes, pub_date: $date, platforms: $p}' > latest.json
cat latest.json
