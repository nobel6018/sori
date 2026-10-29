#!/usr/bin/env bash
# 빌드가 끝난 작업마다 부른다. Release가 없으면 만들고, 받은 파일을 올린다(release.yml).
# 여러 작업이 동시에 만들려 하면 하나만 성공하므로, 만들기에 실패해도 Release가 있으면 그대로 올린다
set -euo pipefail
: "${TAG:?}" "${GITHUB_REPOSITORY:?}"
notes="$(dirname "$0")/../release-notes.md"
if ! gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
  gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --title "sori $TAG" --notes-file "$notes" \
    || gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null
fi
printf '%s\n' "$@"
gh release upload "$TAG" "$@" --repo "$GITHUB_REPOSITORY" --clobber
