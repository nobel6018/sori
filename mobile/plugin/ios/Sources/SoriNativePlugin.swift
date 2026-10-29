// sori iPhone 앱의 네이티브 기능. Rust(src/lib.rs)가 run_mobile_plugin으로 부른다.
// @see docs/mobile.md

import AVFoundation
import SwiftRs
import Tauri
import UIKit
import UniformTypeIdentifiers
import WebKit

class PathArgs: Decodable {
  let path: String
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

  // MARK: 녹음

  @objc public func startRecording(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PathArgs.self)
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
          guard recorder.record() else {
            invoke.reject("녹음을 시작하지 못했습니다")
            return
          }
          self.recorder = recorder
          invoke.resolve()
        } catch {
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

@_cdecl("init_plugin_sori_native")
public func initPlugin() -> Plugin {
  return SoriNativePlugin()
}
