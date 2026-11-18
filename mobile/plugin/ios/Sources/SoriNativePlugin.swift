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

class ConfirmArgs: Decodable {
  let title: String
  let message: String?
  /// 확인 버튼 글자(예: "지우기")
  let ok: String
  /// 확인 버튼을 빨갛게(지우기처럼 되돌릴 수 없는 일)
  let destructive: Bool?
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
  private var recorder: SegmentedRecorder?
  /// 위치를 받는 동안 붙잡아 둔다. 놓으면 CLLocationManager가 사라져 답이 오지 않는다
  private var locationFetcher: LocationFetcher?
  private var pendingPick: Invoke?
  /// 지금 접근 권한을 연 동기화 폴더. 앱이 켜져 있는 동안 열어 둔다
  private var openFolder: URL?
  private let bookmarkKey = "sori.syncFolderBookmark"

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
        let recorder = SegmentedRecorder(audioPath: args.path, background: args.background == true)
        do {
          try recorder.start()
          self.recorder = recorder
          invoke.resolve()
        } catch {
          // 세션이 켜진 채 남으면 다음 시작의 카테고리 설정을 iOS가 다른 앱 소리를 끊으려는 시도로 보고 !int로
          // 거절해, 실패 한 번이 뒤의 시도를 줄줄이 실패시켰다(0.7.5). 실패하면 꼭 끈다
          _ = recorder.stop()
          invoke.reject("녹음을 시작하지 못했습니다: \(error.localizedDescription)")
        }
      }
    }
  }

  /// 녹음을 멈추고 녹음한 길이를 돌려준다. 구간 파일을 audio.m4a로 합치는 것은 finalizeRecording이 따로 한다
  @objc public func stopRecording(_ invoke: Invoke) {
    onMain {
      guard let recorder = self.recorder else {
        invoke.resolve(["duration": 0, "gaps": []])
        return
      }
      self.recorder = nil
      let duration = recorder.stop()
      invoke.resolve(["duration": duration, "gaps": recorder.gaps])
    }
  }

  @objc public func recorderState(_ invoke: Invoke) {
    onMain {
      guard let recorder = self.recorder else {
        invoke.resolve(["recording": false, "paused": false, "time": 0, "level": 0])
        return
      }
      invoke.resolve([
        "recording": recorder.isRecording, "paused": recorder.paused, "time": recorder.time, "level": recorder.level,
      ])
    }
  }

  /// 구간 WAV를 audio.m4a로 합친다. 멈춘 뒤, 그리고 앱이 녹음 중에 죽었다 다시 켜졌을 때 부른다.
  /// 1시간 녹음이면 수십 초 걸릴 수 있어 메인 스레드 밖에서 하고, 앱이 뒤로 가도 마치도록 백그라운드 시간을 받는다
  @objc public func finalizeRecording(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
    let audio = URL(fileURLWithPath: args.path)
    // 어느 스레드에서 불러도 된다(Apple 문서). 시간이 다 되면 그대로 두고, 남은 구간은 다음 실행 때 다시 합친다
    let task = UIApplication.shared.beginBackgroundTask(withName: "sori.finalize")
    DispatchQueue.global(qos: .userInitiated).async {
      defer { UIApplication.shared.endBackgroundTask(task) }
      do {
        let (outcome, duration) = try RecordingFiles.finalize(audio: audio)
        invoke.resolve(["outcome": outcome.rawValue, "duration": duration])
      } catch {
        invoke.reject("녹음 파일을 합치지 못했습니다. 녹음 구간은 그대로 남아 있고 다음에 다시 합칩니다: \(error.localizedDescription)")
      }
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

  /// 동기화 폴더에서 다른 기기가 고친 뒤 아직 이 iPhone에 내려오지 않은 작은 파일(meta.json 등)을 받아 온다.
  /// iOS iCloud는 한 번 받은 파일이 클라우드에서 바뀌어도 바로 내려받지 않고 예전 내용을 그대로 둔다(상태 .downloaded).
  /// 동기화 엔진은 파일 수정 시각과 크기로 바뀐 것을 알아채므로, 내려오기 전에는 Mac이 고친 태그를 몰랐다(0.9.5까지).
  /// 오디오는 크므로 건드리지 않는다(필요할 때 materialize). 최대 15초 기다리고, 받은 개수를 돌려준다. @see docs/sync.md
  @objc public func refreshIcloud(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
    DispatchQueue.global(qos: .userInitiated).async {
      let root = URL(fileURLWithPath: args.path)
      let keys: [URLResourceKey] = [.isRegularFileKey, .ubiquitousItemDownloadingStatusKey]
      var stale: [URL] = []
      if let walker = FileManager.default.enumerator(at: root, includingPropertiesForKeys: keys) {
        for case let url as URL in walker {
          let name = url.lastPathComponent
          if name.hasPrefix("audio.") || name.hasSuffix(".tmp") { continue }
          guard let values = try? url.resourceValues(forKeys: Set(keys)), values.isRegularFile == true else { continue }
          if let status = values.ubiquitousItemDownloadingStatus, status != .current {
            stale.append(url)
          }
        }
      }
      let group = DispatchGroup()
      for url in stale {
        try? FileManager.default.startDownloadingUbiquitousItem(at: url)
        group.enter()
        DispatchQueue.global(qos: .userInitiated).async {
          var error: NSError?
          NSFileCoordinator(filePresenter: nil).coordinate(readingItemAt: url, options: [], error: &error) { _ in }
          group.leave()
        }
      }
      _ = group.wait(timeout: .now() + 15)
      invoke.resolve(["count": stale.count])
    }
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
  /// 당겨서 새로고침이 기준선을 넘을 때 iOS 목록과 같은 가벼운 진동. 웹뷰(navigator.vibrate)는 iOS에서 동작하지 않는다
  @objc public func haptic(_ invoke: Invoke) {
    onMain {
      UIImpactFeedbackGenerator(style: .light).impactOccurred()
      invoke.resolve()
    }
  }

  /// iOS 알림창으로 묻는다. 웹뷰(wry)가 iOS에서 window.confirm을 구현하지 않아 confirm()은 창 없이 늘 false였다.
  /// 그래서 녹음 삭제, 동기화 끄기 버튼이 아무 반응이 없었다(0.15.1까지)
  @objc public func confirm(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ConfirmArgs.self)
    onMain {
      guard var top = self.manager.viewController else {
        invoke.reject("확인 창을 띄울 화면이 없습니다")
        return
      }
      while let presented = top.presentedViewController { top = presented }
      let alert = UIAlertController(title: args.title, message: args.message, preferredStyle: .alert)
      alert.addAction(UIAlertAction(title: "취소", style: .cancel) { _ in invoke.resolve(["ok": false]) })
      alert.addAction(
        UIAlertAction(title: args.ok, style: args.destructive == true ? .destructive : .default) { _ in
          invoke.resolve(["ok": true])
        })
      top.present(alert, animated: true)
    }
  }

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
