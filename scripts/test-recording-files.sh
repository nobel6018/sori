#!/bin/sh
# iPhone 녹음 구간 파일의 복구와 합치기(mobile/plugin/ios/Sources/RecordingFiles.swift)를 Mac에서 검사한다.
# 녹음은 iPhone에서만 되지만 이 부분은 UIKit을 쓰지 않아 Mac에서 그대로 돈다. @see docs/recording-safety.md
set -eu
cd "$(dirname "$0")/.."
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
# 여러 파일을 함께 컴파일하면 맨 위 코드는 main.swift에서만 돈다
cp mobile/plugin/ios/Tests/RecordingFilesTests.swift "$out/main.swift"
swiftc -O -o "$out/recording-files-tests" mobile/plugin/ios/Sources/RecordingFiles.swift "$out/main.swift"
"$out/recording-files-tests"
