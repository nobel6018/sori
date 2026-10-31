// 단축어와 동작 버튼으로 녹음을 시작하고 멈춘다. 녹음 상태는 Rust(AppState)가 쥐고, 여기서는 Rust를 부르기만 한다.
// Live Activity는 단축어, 동작 버튼이면 인텐트 안에서 직접 띄운다. iOS는 백그라운드 앱의 Live Activity를 인텐트를 실행하는
// 동안에만 허락한다. 화면 버튼으로 시작하면 Rust가 아래 훅으로 띄운다. 내리는 것은 어디서 시작했든 Rust 훅이 한다.
// @see docs/mobile.md

import ActivityKit
import AppIntents
import Foundation

struct RecordingError: Error, CustomLocalizedStringResourceConvertible {
  let message: String
  var localizedStringResource: LocalizedStringResource { "\(message)" }
}

enum RecordingControl {
  /// 백그라운드에서 먼저 시도하고, iOS가 거절하면 앱을 앞으로 띄워 다시 시작한다.
  /// 백그라운드 녹음 시작은 iOS가 임시 허가를 주고도 가끔 거절한다(원인 미확정, docs/mobile.md). 앞에서는 늘 된다
  static func start(continuing intent: some AppIntent) async throws {
    do {
      try await startInBackground()
    } catch {
      guard #available(iOS 26.0, *) else { throw error }
      NSLog("sori: 백그라운드 녹음 시작 실패, 앱을 앞으로 띄워 다시 시작합니다: \(error)")
      // 화면이 잠겨 있으면 잠금 해제를 요구한다. 거절하면 오류가 그대로 단축어에 간다
      try await intent.continueInForeground(nil, alwaysConfirm: false)
      try await callRust { sori_intent_start_recording() }
    }
  }

  private static func startInBackground() async throws {
    // AudioRecordingIntent는 녹음하는 동안 Live Activity를 띄워야 한다. 못 띄우면 시스템이 녹음을 끊는다
    guard ActivityAuthorizationInfo().areActivitiesEnabled else {
      throw RecordingError(message: "실시간 현황이 꺼져 있어 녹음할 수 없습니다. 설정 > 앱 > sori에서 실시간 현황을 켜 주세요")
    }
    // 오디오 세션보다 먼저 띄운다. 앱 밖에서 시작하면 iOS는 Live Activity가 떠 있을 때만 백그라운드 녹음을 허락한다
    if let error = startLiveActivity() {
      throw RecordingError(message: "Live Activity를 띄우지 못해 녹음하지 않았습니다: \(error)")
    }
    // 실패하면 Rust(begin_recording)가 Live Activity를 내린다
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

/// 띄웠으면 nil, 못 띄웠으면 iOS가 준 오류(visibility, unsupportedTarget 등). 이미 떠 있으면(인텐트가 띄움) 그대로 둔다
private func startLiveActivity() -> String? {
  guard Activity<RecordingActivityAttributes>.activities.isEmpty else { return nil }
  guard ActivityAuthorizationInfo().areActivitiesEnabled else { return "실시간 현황이 꺼져 있습니다" }
  let state = RecordingActivityAttributes.ContentState(startedAt: Date())
  do {
    _ = try Activity.request(attributes: RecordingActivityAttributes(), content: .init(state: state, staleDate: nil))
    return nil
  } catch {
    NSLog("sori: Live Activity를 띄우지 못했습니다: \(error)")
    return "\(error)"
  }
}

/// Rust에 넘긴 마지막 오류 문구. Rust가 바로 복사하므로 다음 호출 때 지운다
private var lastActivityError: UnsafeMutablePointer<CChar>?

/// Rust 훅. 띄웠으면 null, 못 띄웠으면 오류 문구
private func startLiveActivityForRust() -> UnsafePointer<CChar>? {
  free(lastActivityError)
  lastActivityError = nil
  guard let error = startLiveActivity() else { return nil }
  lastActivityError = strdup(error)
  return UnsafePointer(lastActivityError)
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
  sori_set_activity_hooks({ startLiveActivityForRust() }, { endLiveActivities() })
}
