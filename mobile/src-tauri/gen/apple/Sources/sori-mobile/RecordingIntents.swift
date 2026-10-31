// 단축어 앱, 동작 버튼, Siri에 보이는 sori 녹음 동작.
// AudioRecordingIntent(iOS 18)라 앱을 열지 않고 백그라운드에서 녹음을 시작한다. 녹음하는 동안 Live Activity가 떠 있어야 한다.
// LiveActivityIntent도 함께 선언해야 백그라운드에서 Live Activity를 띄울 수 있다. 이게 없던 0.7.2는 단축어 실행기가
// Live Activity 시작 허가(activity request assertion)를 붙이지 않아 visibility 오류로 실패했다
// @see docs/mobile.md

import AppIntents

struct StartRecordingIntent: AudioRecordingIntent, LiveActivityIntent {
  static let title: LocalizedStringResource = "녹음 시작"
  static let description = IntentDescription("앱을 열지 않고 바로 녹음을 시작합니다. 녹음하는 동안 잠금 화면과 Dynamic Island에 경과 시간이 보입니다")

  /// 백그라운드로 시작하고, 필요하면(녹음 시작을 iOS가 거절하면) 앱을 앞으로 띄운다
  @available(iOS 26.0, *)
  static var supportedModes: IntentModes { [.background, .foreground(.dynamic)] }

  func perform() async throws -> some IntentResult {
    try await RecordingControl.start(continuing: self)
    return .result()
  }
}

/// 동작 버튼용. 한 번 누르면 시작, 다시 누르면 멈춘다
struct ToggleRecordingIntent: AudioRecordingIntent, LiveActivityIntent {
  static let title: LocalizedStringResource = "녹음 시작/멈춤"
  static let description = IntentDescription("녹음 중이 아니면 바로 녹음을 시작하고, 녹음 중이면 멈춥니다")

  @available(iOS 26.0, *)
  static var supportedModes: IntentModes { [.background, .foreground(.dynamic)] }

  func perform() async throws -> some IntentResult {
    if await RecordingControl.isRecording() {
      try await RecordingControl.stop()
    } else {
      try await RecordingControl.start(continuing: self)
    }
    return .result()
  }
}

/// 단축어를 따로 만들지 않아도 단축어 앱의 sori 항목과 동작 버튼 목록에 바로 보인다
struct SoriShortcuts: AppShortcutsProvider {
  static var appShortcuts: [AppShortcut] {
    AppShortcut(
      intent: ToggleRecordingIntent(),
      phrases: ["\(.applicationName) 녹음", "\(.applicationName) 녹음 켜고 끄기"],
      shortTitle: "녹음 시작/멈춤",
      systemImageName: "record.circle"
    )
    AppShortcut(
      intent: StartRecordingIntent(),
      phrases: ["\(.applicationName) 녹음 시작", "\(.applicationName)로 녹음 시작"],
      shortTitle: "녹음 시작",
      systemImageName: "mic.circle"
    )
    AppShortcut(
      intent: StopRecordingIntent(),
      phrases: ["\(.applicationName) 녹음 멈춰", "\(.applicationName) 녹음 끝"],
      shortTitle: "녹음 멈추기",
      systemImageName: "stop.circle"
    )
  }
}
