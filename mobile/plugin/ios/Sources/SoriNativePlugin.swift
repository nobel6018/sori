// sori iPhone 앱의 네이티브 기능. Rust(src/lib.rs)가 run_mobile_plugin으로 부른다.
// @see docs/mobile.md

import AVFoundation
import CoreLocation
import SwiftRs
import Tauri
import UIKit
import UniformTypeIdentifiers
import WebKit

class PathArgs: Decodable {
  let path: String
}

class UrlArgs: Decodable {
  let url: String
}

class StartArgs: Decodable {
  let path: String
  /// 단축어, 동작 버튼으로 앱 밖에서 시작했다
  let background: Bool?
}

/// UI와 AVAudioRecorder는 메인 스레드에서 다룬다. 호출한 쪽이 이미 메인 스레드면 바로 실행한다.
/// 메인 스레드에서 Rust가 응답을 기다리는 중에 main.async로 넘기면 서로 기다리며 앱이 멈춘다
private func onMain(_ work: @escaping () -> Void) {
  if Thread.isMainThread {
    work()
  } else {
    DispatchQueue.main.async(execute: work)
  }
}

class SoriNativePlugin: Plugin, UIDocumentPickerDelegate {
  private var recorder: AVAudioRecorder?
  /// 위치를 받는 동안 붙잡아 둔다. 놓으면 CLLocationManager가 사라져 답이 오지 않는다
  private var locationFetcher: LocationFetcher?
  private var pendingPick: Invoke?
  /// 지금 접근 권한을 연 동기화 폴더. 앱이 켜져 있는 동안 열어 둔다
  private var openFolder: URL?
  private let bookmarkKey = "sori.syncFolderBookmark"

  override init() {
    super.init()
    // 전화가 오는 등으로 녹음이 끊겼다가 끝나면 이어서 녹음한다. AVAudioRecorder는 스스로 다시 시작하지 않는다
    NotificationCenter.default.addObserver(
      forName: AVAudioSession.interruptionNotification, object: nil, queue: .main
    ) { [weak self] note in
      guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
        AVAudioSession.InterruptionType(rawValue: raw) == .ended,
        let recorder = self?.recorder
      else { return }
      try? AVAudioSession.sharedInstance().setActive(true)
      recorder.record()
    }
  }

  // MARK: 미리 전사

  /// iPhone에서 바로 미리 전사한다(iOS 26 이상). 못 하면 available: false. 1시간 녹음이면 몇 분 걸린다
  @objc public func transcribePreview(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
    guard #available(iOS 26.0, *) else {
      invoke.resolve(["available": false, "reason": "iOS 26 이상에서 됩니다"])
      return
    }
    Task {
      do {
        let (utterances, duration) = try await runPreviewTranscription(path: args.path)
        invoke.resolve([
          "available": true,
          "duration": duration,
          "utterances": utterances.map { ["start": $0.start, "end": $0.end, "text": $0.text] },
        ])
      } catch {
        invoke.reject("iPhone 미리 전사를 하지 못했습니다: \(error.localizedDescription)")
      }
    }
  }

  // MARK: 위치

  /// 지금 위치와 주소. 못 받으면 found: false. 최대 약 18초
  @objc public func currentLocation(_ invoke: Invoke) {
    onMain {
      let fetcher = LocationFetcher()
      self.locationFetcher = fetcher
      fetcher.fetch(timeout: 10) { location in
        self.locationFetcher = nil
        guard let location else {
          invoke.resolve(["found": false])
          return
        }
        var answered = false
        let send = { (place: String?) in
          if answered { return }
          answered = true
          invoke.resolve([
            "found": true,
            "lat": location.coordinate.latitude,
            "lon": location.coordinate.longitude,
            "accuracy": location.horizontalAccuracy,
            "place": place ?? NSNull(),
          ])
        }
        let geocoder = CLGeocoder()
        geocoder.reverseGeocodeLocation(location) { marks, _ in
          onMain { send(describePlace(marks?.first)) }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 8) {
          geocoder.cancelGeocode()
          send(nil)
        }
      }
    }
  }

  // MARK: 녹음

  @objc public func startRecording(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StartArgs.self)
    AVAudioApplication.requestRecordPermission { granted in
      onMain {
        guard granted else {
          invoke.reject("마이크 권한이 없습니다. 설정 > 앱 > sori에서 마이크를 켜 주세요")
          return
        }
        do {
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
          if args.background == true || UIApplication.shared.applicationState != .active {
            options.insert(.mixWithOthers)
          }
          try session.setCategory(.playAndRecord, mode: .default, options: options)
          try session.setActive(true)
          // 말소리용 AAC 모노 64kbps. 1시간에 약 28MB
          let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 44_100,
            AVNumberOfChannelsKey: 1,
            AVEncoderBitRateKey: 64_000,
          ]
          let recorder = try AVAudioRecorder(url: URL(fileURLWithPath: args.path), settings: settings)
          recorder.isMeteringEnabled = true
          // 백그라운드에서 시작하면 audiomxd가 가끔 녹음 대신 재생 권한 검사("does not have assertions to start
          // mixable playback")로 record()를 거절한다. 같은 세션으로, 세션을 껐다 켜서 다시 해도 소용없었다(0.7.5, 0.7.6).
          // 실패하면 단축어 쪽(RecordingControl.start)이 앱을 앞으로 띄워 다시 시작한다
          guard recorder.record() else {
            try? session.setActive(false)
            invoke.reject("녹음을 시작하지 못했습니다")
            return
          }
          self.recorder = recorder
          invoke.resolve()
        } catch {
          // 세션이 켜진 채 남으면 다음 시작의 카테고리 설정을 iOS가 다른 앱 소리를 끊으려는 시도로 보고 !int로
          // 거절해, 실패 한 번이 뒤의 시도를 줄줄이 실패시켰다(0.7.5). 실패하면 꼭 끈다
          try? AVAudioSession.sharedInstance().setActive(false)
          invoke.reject("녹음을 시작하지 못했습니다: \(error.localizedDescription)")
        }
      }
    }
  }

  @objc public func stopRecording(_ invoke: Invoke) {
    onMain {
      guard let recorder = self.recorder else {
        invoke.resolve(["duration": 0])
        return
      }
      let duration = recorder.currentTime
      recorder.stop()
      self.recorder = nil
      try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
      invoke.resolve(["duration": duration])
    }
  }

  @objc public func recorderState(_ invoke: Invoke) {
    onMain {
      guard let recorder = self.recorder else {
        invoke.resolve(["recording": false, "time": 0, "level": 0])
        return
      }
      recorder.updateMeters()
      // -60dB 이하는 0, 0dB는 1
      let db = recorder.averagePower(forChannel: 0)
      let level = max(0, min(1, (db + 60) / 60))
      invoke.resolve(["recording": recorder.isRecording, "time": recorder.currentTime, "level": level])
    }
  }

  // MARK: iCloud Drive 폴더

  @objc public func pickFolder(_ invoke: Invoke) {
    onMain {
      let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
      picker.delegate = self
      picker.allowsMultipleSelection = false
      self.pendingPick = invoke
      self.manager.viewController?.present(picker, animated: true)
    }
  }

  public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
    guard let invoke = pendingPick else { return }
    pendingPick = nil
    guard let url = urls.first else {
      invoke.resolve(["path": NSNull()])
      return
    }
    guard url.startAccessingSecurityScopedResource() else {
      invoke.reject("폴더에 접근하지 못했습니다")
      return
    }
    do {
      let bookmark = try url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
      UserDefaults.standard.set(bookmark, forKey: bookmarkKey)
      openFolder?.stopAccessingSecurityScopedResource()
      openFolder = url
      invoke.resolve(["path": url.path, "name": url.lastPathComponent])
    } catch {
      url.stopAccessingSecurityScopedResource()
      invoke.reject("폴더 접근 권한을 저장하지 못했습니다: \(error.localizedDescription)")
    }
  }

  public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
    pendingPick?.resolve(["path": NSNull()])
    pendingPick = nil
  }

  @objc public func resolveFolder(_ invoke: Invoke) {
    if let open = openFolder {
      invoke.resolve(["path": open.path, "name": open.lastPathComponent])
      return
    }
    guard let bookmark = UserDefaults.standard.data(forKey: bookmarkKey) else {
      invoke.resolve(["path": NSNull()])
      return
    }
    do {
      var stale = false
      let url = try URL(resolvingBookmarkData: bookmark, options: [], relativeTo: nil, bookmarkDataIsStale: &stale)
      guard url.startAccessingSecurityScopedResource() else {
        invoke.reject("동기화 폴더에 접근하지 못했습니다. 설정에서 폴더를 다시 골라 주세요")
        return
      }
      // 폴더를 옮기거나 이름을 바꾸면 오래된 권한이 된다. 새로 만들어 둔다
      if stale, let fresh = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
        UserDefaults.standard.set(fresh, forKey: bookmarkKey)
      }
      openFolder = url
      invoke.resolve(["path": url.path, "name": url.lastPathComponent])
    } catch {
      invoke.reject("동기화 폴더를 찾지 못했습니다. 설정에서 폴더를 다시 골라 주세요: \(error.localizedDescription)")
    }
  }

  @objc public func forgetFolder(_ invoke: Invoke) {
    openFolder?.stopAccessingSecurityScopedResource()
    openFolder = nil
    UserDefaults.standard.removeObject(forKey: bookmarkKey)
    invoke.resolve()
  }

  /// 받지 않은 iCloud 파일을 받아 온다. 읽기 조정(coordinate reading)은 파일이 기기에 내려올 때까지 기다린다
  @objc public func materialize(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
    DispatchQueue.global(qos: .userInitiated).async {
      let url = URL(fileURLWithPath: args.path)
      try? FileManager.default.startDownloadingUbiquitousItem(at: url)
      var error: NSError?
      NSFileCoordinator(filePresenter: nil).coordinate(readingItemAt: url, options: [], error: &error) { _ in }
      if let error = error {
        invoke.reject(error.localizedDescription)
      } else {
        invoke.resolve()
      }
    }
  }
}

extension SoriNativePlugin {
  /// 지도 앱이나 Safari로 연다. 화면(WebView) 안에서 열면 앱 화면이 그 페이지로 바뀐다
  @objc public func openUrl(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(UrlArgs.self)
    guard let url = URL(string: args.url) else {
      invoke.reject("링크가 잘못됐습니다")
      return
    }
    onMain {
      UIApplication.shared.open(url) { ok in
        if ok { invoke.resolve() } else { invoke.reject("링크를 열지 못했습니다") }
      }
    }
  }
}

@_cdecl("init_plugin_sori_native")
public func initPlugin() -> Plugin {
  return SoriNativePlugin()
}
