// 녹음을 구간 WAV 파일로 나눠 쓰고, 멈춘 뒤 audio.m4a 하나로 합친다.
// 어떤 경우에도 녹음한 소리를 잃지 않는 것이 목적이다. @see docs/recording-safety.md
//
// - 전화 같은 인터럽션이 오면 iOS는 AVAudioRecorder를 멈춘다(일시정지가 아니다). 그 녹음기에 record()를 다시 부르면
//   같은 파일을 처음부터 새로 써서 앞부분이 사라졌다(2026-11-18 #43, #46). 그래서 재개는 늘 새 구간 파일로 한다
// - m4a는 stop()이 끝에 목차(moov)를 써야 읽힌다. 앱이 죽으면 통째로 못 읽는다. WAV는 앞에서부터 그대로 읽혀
//   앱이 죽어도 쓴 데까지 남고, 헤더의 길이만 고치면 된다
// - 합친 m4a의 길이를 확인한 뒤에만 구간 파일을 지운다. 실패하면 구간을 남기고 다음 실행 때 다시 합친다
// - 전화가 울리기만 해도 녹음이 끊겼다(2026-11-18 #48, 받지 않은 전화로 15초). 울림은 인터럽션으로 받지 않는다.
//   전화를 받으면 iOS가 마이크를 통화에 넘겨 녹음할 수 없다. 그 구간은 길이와 이유를 남겨 메모로 보여 준다

import AVFoundation
import CallKit
import UIKit
import UserNotifications

final class SegmentedRecorder: NSObject, AVAudioRecorderDelegate {
  private let partsDir: URL
  private let background: Bool
  private var recorder: AVAudioRecorder?
  /// 닫힌 구간들의 길이 합(초)
  private var closedDuration: TimeInterval = 0
  /// 인터럽션이나 시스템 사정으로 멈춰 있고, 사람이 멈춘 것은 아니다
  private(set) var paused = false
  /// stop()을 불렀다. 이후 오는 알림은 무시한다
  private var ended = false
  private var observers: [NSObjectProtocol] = []
  /// 지금 멈춘 구간이 시작된 때와 이유. 다시 이어지면 gaps에 넣는다
  private var pausedSince: (date: Date, reason: String)?
  /// 녹음이 멈췄다 이어진 곳들. time은 이어 붙인 오디오에서 끊긴 위치(초), seconds는 녹음하지 못한 시간
  private(set) var gaps: [[String: Any]] = []
  /// 진행 중인 전화가 있는지 본다. 권한(entitlement) 없이 쓸 수 있다
  private let callObserver = CXCallObserver()

  static let pausedNotificationId = "sori.recording.paused"

  init(audioPath: String, background: Bool) {
    partsDir = RecordingFiles.partsDir(for: URL(fileURLWithPath: audioPath))
    self.background = background
    super.init()
  }

  // MARK: 시작, 멈춤, 상태

  func start() throws {
    try FileManager.default.createDirectory(at: partsDir, withIntermediateDirectories: true)
    try configureSession()
    try startSegment()
    observe()
    requestNotificationPermission()
  }

  /// 녹음한 길이(초)를 돌려준다. 구간 파일은 그대로 두고, 합치기는 finalize가 한다
  func stop() -> TimeInterval {
    ended = true
    observers.forEach { NotificationCenter.default.removeObserver($0) }
    observers = []
    closeSegment()
    try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.pausedNotificationId])
    UNUserNotificationCenter.current().removePendingNotificationRequests(withIdentifiers: [Self.pausedNotificationId])
    return closedDuration
  }

  var isRecording: Bool { recorder?.isRecording ?? false }

  /// 지금까지 녹음한 시간. 메모 시각도 이 값을 쓴다(구간을 이어 붙인 오디오의 시각과 같다)
  var time: TimeInterval { closedDuration + (recorder?.currentTime ?? 0) }

  var level: Float {
    guard let recorder, recorder.isRecording else { return 0 }
    recorder.updateMeters()
    // -60dB 이하는 0, 0dB는 1
    return max(0, min(1, (recorder.averagePower(forChannel: 0) + 60) / 60))
  }

  // MARK: 구간

  private func configureSession() throws {
    let session = AVAudioSession.sharedInstance()
    // 블루투스 이어폰 마이크도 쓰고, 녹음 중에도 재생 소리가 스피커로 나오게 한다
    // iOS 26에서 allowBluetooth가 allowBluetoothHFP로 이름이 바뀌었다
    var options: AVAudioSession.CategoryOptions = [.defaultToSpeaker]
    if #available(iOS 26.0, *) {
      options.insert(.allowBluetoothHFP)
    } else {
      options.insert(.allowBluetooth)
    }
    // 동작 버튼, 단축어로 백그라운드에서 시작하면 다른 앱의 소리를 멈출 수 없어 세션 켜기가 실패한다
    // (audiomxd: "can't stop others because it is in the background", -12985). 이때는 섞어서 켠다.
    // applicationState만 보면 정지에서 막 깨어난 앱이 active로 나와 섞지 않는 세션으로 실패했다(0.7.4)
    if background || UIApplication.shared.applicationState != .active {
      options.insert(.mixWithOthers)
    }
    try session.setCategory(.playAndRecord, mode: .default, options: options)
    // 전화가 울리기만 할 때는 녹음을 끊지 않는다. 받으면 그때 인터럽션이 온다(iOS 14.5, 녹음 앱용 옵션)
    try? session.setPrefersNoInterruptionsFromSystemAlerts(true)
    try session.setActive(true)
  }

  private func nextSegmentURL() -> URL {
    var index = RecordingFiles.segmentFiles(in: partsDir).count + 1
    while true {
      let url = partsDir.appendingPathComponent(String(format: "%04d.wav", index))
      if !FileManager.default.fileExists(atPath: url.path) { return url }
      index += 1
    }
  }

  private func startSegment() throws {
    let settings: [String: Any] = [
      AVFormatIDKey: kAudioFormatLinearPCM,
      AVSampleRateKey: RecordingFormat.sampleRate,
      AVNumberOfChannelsKey: 1,
      AVLinearPCMBitDepthKey: 16,
      AVLinearPCMIsFloatKey: false,
      AVLinearPCMIsBigEndianKey: false,
    ]
    // 늘 새 녹음기, 새 파일. 멈춘 녹음기를 다시 쓰면 파일을 처음부터 새로 쓴다
    let recorder = try AVAudioRecorder(url: nextSegmentURL(), settings: settings)
    recorder.delegate = self
    recorder.isMeteringEnabled = true
    // 백그라운드에서 시작하면 audiomxd가 가끔 녹음 대신 재생 권한 검사("does not have assertions to start
    // mixable playback")로 record()를 거절한다. 단축어 쪽(RecordingControl.start)이 앱을 앞으로 띄워 다시 시작한다
    guard recorder.record() else {
      throw RecordingFailure("녹음을 시작하지 못했습니다")
    }
    self.recorder = recorder
    paused = false
  }

  /// 지금 구간을 닫는다. 녹음기가 이미 시스템에 의해 멈췄어도 stop()으로 헤더를 마저 쓴다
  private func closeSegment() {
    if let recorder {
      self.recorder = nil
      recorder.delegate = nil
      recorder.stop()
    }
    closedDuration = RecordingFiles.segmentFiles(in: partsDir).reduce(0) { $0 + (WavFile.duration(of: $1) ?? 0) }
  }

  // MARK: 인터럽션과 재개

  private func observe() {
    let center = NotificationCenter.default
    observers.append(
      center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
        self?.handleInterruption(note)
      })
    // 앱이 앞으로 오면 재개는 늘 된다. 통화 뒤 백그라운드 재개가 실패했거나 끝났다는 알림이 오지 않았을 때 여기서 잇는다
    observers.append(
      center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
        guard let self, self.paused else { return }
        NSLog("sori: 앱이 앞으로 와서 멈춘 녹음을 잇습니다")
        self.resume()
      })
    // 오디오 서버가 다시 시작되면 녹음기와 세션이 모두 무효가 된다
    observers.append(
      center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, !self.ended else { return }
        NSLog("sori: 오디오 서비스가 다시 시작돼 새 구간으로 잇습니다")
        self.closeSegment()
        self.paused = true
        self.markPaused(reason: "오디오 서비스 재시작")
        try? self.configureSession()
        self.resume()
      })
  }

  private func handleInterruption(_ note: Notification) {
    guard !ended, let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
      let type = AVAudioSession.InterruptionType(rawValue: raw)
    else { return }
    switch type {
    case .began:
      NSLog("sori: 인터럽션 시작, 지금 구간을 닫습니다")
      closeSegment()
      paused = true
      markPaused(reason: onCall ? "통화" : "다른 앱 소리")
    case .ended:
      // shouldResume이 없어도 잇는다. 녹음을 멈춘 건 사람이 아니다
      NSLog("sori: 인터럽션 끝, 새 구간으로 잇습니다")
      resume()
    @unknown default:
      break
    }
  }

  private func resume() {
    guard paused, !ended else { return }
    // 통화가 끝난 직후 앱은 잠깐만 깨어 있다. 재개와 확인을 마칠 시간을 받는다
    let task = UIApplication.shared.beginBackgroundTask(withName: "sori.resume")
    defer {
      DispatchQueue.main.asyncAfter(deadline: .now() + 3) { UIApplication.shared.endBackgroundTask(task) }
    }
    do {
      try AVAudioSession.sharedInstance().setActive(true)
      try startSegment()
      recordGap()
    } catch {
      NSLog("sori: 녹음을 잇지 못했습니다: \(error)")
      paused = true
      notifyPaused()
      return
    }
    // 통화 뒤 백그라운드 재개는 iOS가 허락하고도 0.15초 만에 끊은 적이 있다(2026-11-18). 실제로 이어지는지 확인한다
    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in self?.verifyRunning() }
  }

  private func verifyRunning() {
    guard !ended, let recorder else { return }
    if recorder.isRecording && recorder.currentTime > 0.5 {
      UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.pausedNotificationId])
      return
    }
    NSLog("sori: 이은 녹음이 멈춰 있습니다(isRecording \(recorder.isRecording), \(recorder.currentTime)초)")
    closeSegment()
    paused = true
    markPaused(reason: onCall ? "통화" : "다른 앱 소리")
    notifyPaused()
  }

  func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
    // 우리가 닫은 구간은 delegate를 먼저 떼므로 여기 오지 않는다. 오면 시스템이 멈춘 것이다
    guard recorder === self.recorder, !ended else { return }
    NSLog("sori: 녹음기가 스스로 멈췄습니다(성공 \(flag))")
    closeSegment()
    paused = true
    markPaused(reason: onCall ? "통화" : "녹음 오류")
    if UIApplication.shared.applicationState == .active {
      resume()
    } else {
      notifyPaused()
    }
  }

  func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: Error?) {
    guard recorder === self.recorder, !ended else { return }
    NSLog("sori: 녹음 중 오류: \(String(describing: error))")
    closeSegment()
    paused = true
    markPaused(reason: "녹음 오류")
    notifyPaused()
  }

  // MARK: 빠진 구간

  private var onCall: Bool { callObserver.calls.contains { !$0.hasEnded } }

  /// 처음 멈춘 때만 남긴다. 재개가 실패해 다시 멈춰도 같은 빈 구간으로 본다
  private func markPaused(reason: String) {
    if pausedSince == nil { pausedSince = (Date(), reason) }
  }

  /// 새 구간이 시작됐다. 멈춰 있던 시간을 남긴다
  private func recordGap() {
    guard let since = pausedSince else { return }
    pausedSince = nil
    let seconds = Date().timeIntervalSince(since.date)
    guard seconds >= 1 else { return }
    gaps.append(["time": closedDuration, "seconds": seconds, "reason": since.reason])
  }

  // MARK: 알림

  private func requestNotificationPermission() {
    UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
  }

  private func notifyPaused() {
    let content = UNMutableNotificationContent()
    content.title = "녹음이 멈췄습니다"
    content.body = "통화나 다른 앱 때문에 녹음이 멈췄습니다. 눌러서 sori를 열면 이어서 녹음합니다. 지금까지 녹음한 부분은 저장돼 있습니다."
    content.sound = .default
    let request = UNNotificationRequest(identifier: Self.pausedNotificationId, content: content, trigger: nil)
    UNUserNotificationCenter.current().add(request)
  }
}
