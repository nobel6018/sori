// Live Activity의 멈춤 버튼과 단축어 "녹음 멈추기".
// 버튼은 위젯 확장이 그리지만 실행은 앱 프로세스에서 한다(LiveActivityIntent). 그래서 위젯 쪽에서는 Rust 호출을 뺀다.
// @see docs/mobile.md

import AppIntents

struct StopRecordingIntent: LiveActivityIntent {
  static let title: LocalizedStringResource = "녹음 멈추기"
  static let description = IntentDescription("sori 녹음을 멈추고 iCloud Drive로 올립니다")

  func perform() async throws -> some IntentResult {
    #if !WIDGET_EXTENSION
      try await RecordingControl.stop()
    #endif
    return .result()
  }
}
