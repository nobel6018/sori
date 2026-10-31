// 단축어와 동작 버튼으로 녹음을 시작하고 멈춘다. 녹음 상태는 Rust(AppState)가 쥐고, 여기서는 Rust를 부르기만 한다.
// Live Activity는 녹음을 어디서 시작했든(화면 버튼 포함) Rust가 아래 훅으로 띄우고 내린다.
// @see docs/mobile.md

import ActivityKit
import Foundation

struct RecordingError: Error, CustomLocalizedStringResourceConvertible {
  let message: String
  var localizedStringResource: LocalizedStringResource { "\(message)" }
}

enum RecordingControl {
  static func start() async throws {
    // AudioRecordingIntent는 녹음하는 동안 Live Activity를 띄워야 한다. 못 띄우면 시스템이 녹음을 끊는다
    guard ActivityAuthorizationInfo().areActivitiesEnabled else {
      throw RecordingError(message: "실시간 현황이 꺼져 있어 녹음할 수 없습니다. 설정 > 앱 > sori에서 실시간 현황을 켜 주세요")
    }
    try await callRust { sori_intent_start_recording() }
  }

  static func stop() async throws {
    try await callRust { sori_intent_stop_recording() }
  }

  static func isRecording() async -> Bool {
    await offMain { sori_intent_is_recording() }
  }

  /// Rust는 메인 스레드에서 도는 Swift 녹음기를 기다리므로, 메인 스레드 밖에서 부른다
  private static func offMain<T>(_ work: @escaping () -> T) async -> T {
    await withCheckedContinuation { continuation in
      DispatchQueue.global(qos: .userInitiated).async { continuation.resume(returning: work()) }
    }
  }

  private static func callRust(_ call: @escaping () -> UnsafeMutablePointer<CChar>?) async throws {
    let message: String? = await offMain {
      guard let pointer = call() else { return nil }
      defer { sori_intent_free(pointer) }
      return String(cString: pointer)
    }
    if let message { throw RecordingError(message: message) }
  }
}

/// 띄웠으면 true. Rust가 녹음(오디오 세션)을 켜기 전에 부른다
private func startLiveActivity() -> Bool {
  guard ActivityAuthorizationInfo().areActivitiesEnabled else { return false }
  let state = RecordingActivityAttributes.ContentState(startedAt: Date())
  do {
    _ = try Activity.request(attributes: RecordingActivityAttributes(), content: .init(state: state, staleDate: nil))
    return true
  } catch {
    NSLog("sori: Live Activity를 띄우지 못했습니다: \(error)")
    return false
  }
}

/// 녹음을 멈춘 인텐트가 끝나기 전에 내려가도록 끝날 때까지(최대 2초) 기다린다. Rust의 작업 스레드에서 불린다
private func endLiveActivities() {
  let activities = Activity<RecordingActivityAttributes>.activities
  guard !activities.isEmpty else { return }
  let done = DispatchSemaphore(value: 0)
  Task.detached {
    for activity in activities {
      await activity.end(nil, dismissalPolicy: .immediate)
    }
    done.signal()
  }
  _ = done.wait(timeout: .now() + 2)
}

/// main.mm이 앱을 띄우기 전에 부른다
@_cdecl("sori_install_live_activity")
public func installLiveActivity() {
  sori_set_activity_hooks({ startLiveActivity() }, { endLiveActivities() })
}
