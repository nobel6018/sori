// 녹음 구간 파일(WAV)을 다루고 audio.m4a로 합친다. UIKit 없이 Mac에서도 테스트한다(scripts/test-recording-files.sh).
// @see docs/recording-safety.md

import AVFoundation
import Foundation

enum RecordingFormat {
  /// 전사 엔진(whisper)이 16kHz로 읽는다. 구간 WAV는 1시간에 약 115MB, 합친 m4a는 약 15MB
  static let sampleRate: Double = 16_000
  static let aacBitRate = 32_000
}

enum RecordingFiles {
  static func partsDir(for audio: URL) -> URL {
    audio.deletingLastPathComponent().appendingPathComponent("parts", isDirectory: true)
  }

  static func segmentFiles(in dir: URL) -> [URL] {
    let names = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
    return names.filter { $0.hasSuffix(".wav") }.sorted().map { dir.appendingPathComponent($0) }
  }

  enum Outcome: String {
    /// audio.m4a가 준비됐다
    case done
    /// 녹음된 소리가 없다(구간도 audio.m4a도 비었다). 구간 파일은 그대로 둔다
    case empty
    /// 구간 없이 옛 형식 audio.m4a만 있는데 읽히지 않는다(0.15.0 이하에서 끊긴 녹음)
    case broken
  }

  /// 구간 WAV를 audio.m4a로 합친다. 다시 해도 안전하다(멱등). 디스크 부족 같은 오류는 throw하고 구간을 남긴다
  static func finalize(audio: URL) throws -> (Outcome, TimeInterval) {
    let dir = partsDir(for: audio)
    let parts = segmentFiles(in: dir)
    let fm = FileManager.default
    if parts.isEmpty {
      // 이미 합쳤거나 옛 형식 녹음
      guard fm.fileExists(atPath: audio.path) else { return (.empty, 0) }
      guard let file = try? AVAudioFile(forReading: audio), file.length > 0 else { return (.broken, 0) }
      return (.done, Double(file.length) / file.fileFormat.sampleRate)
    }
    for part in parts {
      // 앱이 녹음 중에 죽으면 WAV 헤더의 길이가 0으로 남는다. 파일 크기에 맞춰 고친다
      try WavFile.repair(part)
    }
    let usable = parts.filter { (WavFile.duration(of: $0) ?? 0) > 0 }
    guard !usable.isEmpty else { return (.empty, 0) }

    let tmp = audio.deletingLastPathComponent().appendingPathComponent("audio.finishing.m4a")
    try? fm.removeItem(at: tmp)
    let frames = try encode(usable, to: tmp)
    let seconds = Double(frames) / RecordingFormat.sampleRate

    // 다시 열어 길이를 확인한다. AAC 앞뒤 여유 프레임이 있어 1초까지는 같다고 본다
    let check = try AVAudioFile(forReading: tmp)
    let written = Double(check.length) / check.fileFormat.sampleRate
    guard check.length > 0, abs(written - seconds) < 1 else {
      try? fm.removeItem(at: tmp)
      throw RecordingFailure("합친 파일 길이가 맞지 않습니다(구간 \(seconds)초, 파일 \(written)초)")
    }
    if fm.fileExists(atPath: audio.path) {
      // 지우지 않고 옆에 둔다. "audio."로 시작하면 동기화가 오디오로 볼 수 있어 이름을 다르게 한다
      let keep = audio.deletingLastPathComponent().appendingPathComponent("previous-\(Int(Date().timeIntervalSince1970)).m4a")
      try fm.moveItem(at: audio, to: keep)
    }
    try fm.moveItem(at: tmp, to: audio)
    try? fm.removeItem(at: dir)
    return (.done, seconds)
  }

  /// 구간들을 차례로 읽어 AAC로 쓴다. 쓴 프레임 수. 함수가 끝나면 출력 파일이 닫히며 목차(moov)를 쓴다
  private static func encode(_ parts: [URL], to output: URL) throws -> AVAudioFramePosition {
    let settings: [String: Any] = [
      AVFormatIDKey: kAudioFormatMPEG4AAC,
      AVSampleRateKey: RecordingFormat.sampleRate,
      AVNumberOfChannelsKey: 1,
      AVEncoderBitRateKey: RecordingFormat.aacBitRate,
    ]
    let out = try AVAudioFile(forWriting: output, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
    var total: AVAudioFramePosition = 0
    for part in parts {
      let input = try AVAudioFile(forReading: part, commonFormat: .pcmFormatFloat32, interleaved: false)
      guard input.processingFormat.sampleRate == out.processingFormat.sampleRate,
        input.processingFormat.channelCount == out.processingFormat.channelCount
      else {
        throw RecordingFailure("구간 형식이 다릅니다: \(part.lastPathComponent) \(input.processingFormat)")
      }
      guard let buffer = AVAudioPCMBuffer(pcmFormat: input.processingFormat, frameCapacity: 32_768) else {
        throw RecordingFailure("버퍼를 만들지 못했습니다")
      }
      while input.framePosition < input.length {
        try input.read(into: buffer)
        if buffer.frameLength == 0 { break }
        try out.write(from: buffer)
        total += AVAudioFramePosition(buffer.frameLength)
      }
    }
    if #available(iOS 18.0, macOS 15.0, *) {
      out.close()
    }
    return total
  }
}

struct RecordingFailure: LocalizedError {
  let message: String
  init(_ message: String) { self.message = message }
  var errorDescription: String? { message }
}

/// AVAudioRecorder가 쓰는 PCM WAV를 다룬다. 앱이 녹음 중에 죽으면 RIFF와 data 길이가 0으로 남는다
enum WavFile {
  struct Layout {
    let dataOffset: UInt64
    let dataSizeField: UInt64
    let sampleRate: UInt32
    let blockAlign: UInt16
  }

  /// 헤더를 읽는다. data 청크의 길이 값은 믿지 않고 위치만 쓴다
  static func layout(of url: URL) -> Layout? {
    guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
    defer { try? handle.close() }
    guard let head = try? handle.read(upToCount: 64 * 1024), head.count >= 12,
      head.prefix(4) == Data("RIFF".utf8), head[8..<12] == Data("WAVE".utf8)
    else { return nil }
    var offset = 12
    var sampleRate: UInt32 = 0
    var blockAlign: UInt16 = 0
    while offset + 8 <= head.count {
      let id = head[offset..<offset + 4]
      let size = Int(head.uint32(at: offset + 4))
      let body = offset + 8
      if id == Data("fmt ".utf8), body + 14 <= head.count {
        sampleRate = head.uint32(at: body + 4)
        blockAlign = head.uint16(at: body + 12)
      }
      if id == Data("data".utf8) {
        guard sampleRate > 0, blockAlign > 0 else { return nil }
        return Layout(dataOffset: UInt64(body), dataSizeField: UInt64(size), sampleRate: sampleRate, blockAlign: blockAlign)
      }
      // 청크는 짝수 바이트로 맞춘다
      offset = body + size + (size & 1)
    }
    return nil
  }

  /// 소리 데이터의 실제 바이트 수. 헤더 값이 파일 끝까지 다 덮으면(정상 종료) 그 값을,
  /// 헤더 값이 모자라고 그 뒤가 다른 청크가 아니면(녹음 중 종료) 파일 끝까지를 쓴다. 블록 크기로 내림한다
  static func dataBytes(_ layout: Layout, url: URL, fileSize size: UInt64) -> UInt64 {
    let available = size > layout.dataOffset ? size - layout.dataOffset : 0
    let field = layout.dataSizeField
    var bytes = available
    if field > 0, field < available, followedByChunk(url, at: layout.dataOffset + field) {
      bytes = field
    }
    return bytes / UInt64(layout.blockAlign) * UInt64(layout.blockAlign)
  }

  /// 그 위치에 청크 이름(ASCII 4글자)이 있는가. 정상 WAV는 data 뒤에 LIST 같은 청크가 붙기도 한다
  private static func followedByChunk(_ url: URL, at offset: UInt64) -> Bool {
    guard let handle = try? FileHandle(forReadingFrom: url) else { return false }
    defer { try? handle.close() }
    // 청크는 짝수 바이트에서 시작한다
    guard (try? handle.seek(toOffset: offset + (offset & 1))) != nil, let id = try? handle.read(upToCount: 4), id.count == 4
    else { return false }
    return id.allSatisfy { (0x20...0x7E).contains($0) }
  }

  /// 소리 길이(초). 헤더가 깨졌으면 nil
  static func duration(of url: URL) -> TimeInterval? {
    guard let layout = layout(of: url), let size = fileSize(url) else { return nil }
    let frames = dataBytes(layout, url: url, fileSize: size) / UInt64(layout.blockAlign)
    return Double(frames) / Double(layout.sampleRate)
  }

  /// 헤더의 길이 값을 실제 소리 길이에 맞춘다. 맞으면 건드리지 않는다. 파일 내용은 지우지 않는다
  static func repair(_ url: URL) throws {
    guard let layout = layout(of: url), let size = fileSize(url) else { return }
    let bytes = dataBytes(layout, url: url, fileSize: size)
    guard layout.dataSizeField != bytes else { return }
    let riff = layout.dataOffset + bytes - 8
    guard bytes <= UInt64(UInt32.max), riff <= UInt64(UInt32.max) else {
      throw RecordingFailure("구간 파일이 4GB를 넘었습니다: \(url.lastPathComponent)")
    }
    NSLog("sori: WAV 헤더를 고칩니다 \(url.lastPathComponent): data \(layout.dataSizeField) -> \(bytes)")
    let handle = try FileHandle(forUpdating: url)
    defer { try? handle.close() }
    try handle.seek(toOffset: 4)
    try handle.write(contentsOf: UInt32(riff).littleEndianData)
    try handle.seek(toOffset: layout.dataOffset - 4)
    try handle.write(contentsOf: UInt32(bytes).littleEndianData)
    try handle.synchronize()
  }

  private static func fileSize(_ url: URL) -> UInt64? {
    (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.uint64Value
  }
}

extension Data {
  fileprivate func uint32(at i: Int) -> UInt32 {
    let s = startIndex + i
    return UInt32(self[s]) | UInt32(self[s + 1]) << 8 | UInt32(self[s + 2]) << 16 | UInt32(self[s + 3]) << 24
  }

  fileprivate func uint16(at i: Int) -> UInt16 {
    let s = startIndex + i
    return UInt16(self[s]) | UInt16(self[s + 1]) << 8
  }
}

extension UInt32 {
  fileprivate var littleEndianData: Data {
    withUnsafeBytes(of: self.littleEndian) { Data($0) }
  }
}
