// RecordingFiles(구간 WAV 복구와 합치기)를 Mac에서 검사한다. scripts/test-recording-files.sh가 RecordingFiles.swift와
// 함께 컴파일해 돌린다. 녹음을 어떤 경우에도 잃지 않는다는 약속을 확인하는 것이 목적이다. @see docs/recording-safety.md

import AVFoundation
import Foundation

var failures = 0

func check(_ ok: Bool, _ message: String, file: StaticString = #file, line: UInt = #line) {
  if ok {
    print("  ok   \(message)")
  } else {
    failures += 1
    print("  FAIL \(message) (\(line))")
  }
}

func near(_ a: Double, _ b: Double, _ tolerance: Double = 0.2) -> Bool { abs(a - b) <= tolerance }

func freshDir(_ name: String) -> URL {
  let dir = FileManager.default.temporaryDirectory.appendingPathComponent("sori-rf-\(name)-\(UUID().uuidString)")
  try! FileManager.default.createDirectory(at: dir.appendingPathComponent("parts"), withIntermediateDirectories: true)
  return dir
}

/// AVAudioRecorder와 같은 형식(16kHz 모노 16비트 PCM WAV)으로 사인파를 쓴다
func writeWav(_ url: URL, seconds: Double, frequency: Double = 440) {
  let settings: [String: Any] = [
    AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: RecordingFormat.sampleRate, AVNumberOfChannelsKey: 1,
    AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
  ]
  let file = try! AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
  let frames = AVAudioFrameCount(seconds * RecordingFormat.sampleRate)
  let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames)!
  buffer.frameLength = frames
  for i in 0..<Int(frames) {
    buffer.floatChannelData![0][i] = Float(0.3 * sin(2 * .pi * frequency * Double(i) / RecordingFormat.sampleRate))
  }
  try! file.write(from: buffer)
  if #available(macOS 15.0, *) { file.close() }
}

/// 앱이 녹음 중에 죽은 파일처럼 RIFF와 data 길이를 0으로 만든다
func zeroHeader(_ url: URL) {
  let layout = WavFile.layout(of: url)!
  let h = try! FileHandle(forUpdating: url)
  try! h.seek(toOffset: 4)
  try! h.write(contentsOf: Data([0, 0, 0, 0]))
  try! h.seek(toOffset: layout.dataOffset - 4)
  try! h.write(contentsOf: Data([0, 0, 0, 0]))
  try! h.close()
}

func setDataField(_ url: URL, _ value: UInt32) {
  let layout = WavFile.layout(of: url)!
  let h = try! FileHandle(forUpdating: url)
  try! h.seek(toOffset: layout.dataOffset - 4)
  try! h.write(contentsOf: withUnsafeBytes(of: value.littleEndian) { Data($0) })
  try! h.close()
}

func m4aSeconds(_ url: URL) -> Double? {
  guard let f = try? AVAudioFile(forReading: url) else { return nil }
  return Double(f.length) / f.fileFormat.sampleRate
}

func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path) }

// 1
print("정상 종료한 구간 두 개를 합친다")
do {
  let dir = freshDir("normal")
  writeWav(dir.appendingPathComponent("parts/0001.wav"), seconds: 3)
  writeWav(dir.appendingPathComponent("parts/0002.wav"), seconds: 2, frequency: 660)
  let audio = dir.appendingPathComponent("audio.m4a")
  let (outcome, seconds) = try! RecordingFiles.finalize(audio: audio)
  check(outcome == .done, "결과 done")
  check(near(seconds, 5), "길이 5초 (\(seconds))")
  check(near(m4aSeconds(audio) ?? 0, 5), "audio.m4a를 다시 열면 5초 (\(m4aSeconds(audio) ?? -1))")
  check(!exists(dir.appendingPathComponent("parts")), "확인 뒤 구간 폴더를 지운다")
  check(!exists(dir.appendingPathComponent("audio.finishing.m4a")), "임시 파일이 남지 않는다")

  print("다시 합쳐도 같다(멱등)")
  let (again, againSeconds) = try! RecordingFiles.finalize(audio: audio)
  check(again == .done && near(againSeconds, 5), "다시 불러도 done, 5초 (\(againSeconds))")
}

// 2
print("앱이 녹음 중에 죽어 헤더 길이가 0인 구간을 살린다")
do {
  let dir = freshDir("crash")
  let part = dir.appendingPathComponent("parts/0001.wav")
  writeWav(part, seconds: 4)
  zeroHeader(part)
  check(near(WavFile.duration(of: part) ?? 0, 4), "헤더가 0이어도 파일 크기로 4초를 센다")
  let (outcome, seconds) = try! RecordingFiles.finalize(audio: dir.appendingPathComponent("audio.m4a"))
  check(outcome == .done && near(seconds, 4), "고쳐서 4초를 합친다 (\(outcome), \(seconds))")
}

// 3
print("헤더 길이가 중간까지만 적힌 구간은 파일 끝까지 쓴다")
do {
  let dir = freshDir("partial")
  let part = dir.appendingPathComponent("parts/0001.wav")
  writeWav(part, seconds: 3)
  setDataField(part, 16_000)  // 0.5초만 적힘
  check(near(WavFile.duration(of: part) ?? 0, 3), "3초로 센다 (\(WavFile.duration(of: part) ?? -1))")
}

// 4
print("data 뒤에 다른 청크가 붙은 정상 파일은 헤더 길이를 믿는다")
do {
  let dir = freshDir("trailing")
  let part = dir.appendingPathComponent("parts/0001.wav")
  writeWav(part, seconds: 2)
  let h = try! FileHandle(forUpdating: part)
  try! h.seekToEnd()
  var list = Data("LIST".utf8)
  list.append(contentsOf: [4, 0, 0, 0])
  list.append(Data("INFO".utf8))
  try! h.write(contentsOf: list)
  try! h.close()
  check(near(WavFile.duration(of: part) ?? 0, 2, 0.001), "LIST를 소리로 세지 않는다 (\(WavFile.duration(of: part) ?? -1))")
}

// 5
print("인터럽션 사이 빈 구간이 섞여도 소리가 있는 구간만 합친다")
do {
  let dir = freshDir("emptypart")
  writeWav(dir.appendingPathComponent("parts/0001.wav"), seconds: 2)
  writeWav(dir.appendingPathComponent("parts/0002.wav"), seconds: 0)
  writeWav(dir.appendingPathComponent("parts/0003.wav"), seconds: 1)
  let (outcome, seconds) = try! RecordingFiles.finalize(audio: dir.appendingPathComponent("audio.m4a"))
  check(outcome == .done && near(seconds, 3), "3초 (\(outcome), \(seconds))")
}

// 6
print("소리가 하나도 없으면 empty, 구간 파일은 남긴다")
do {
  let dir = freshDir("empty")
  writeWav(dir.appendingPathComponent("parts/0001.wav"), seconds: 0)
  let (outcome, _) = try! RecordingFiles.finalize(audio: dir.appendingPathComponent("audio.m4a"))
  check(outcome == .empty, "결과 empty")
  check(exists(dir.appendingPathComponent("parts/0001.wav")), "구간 파일을 지우지 않는다")
}

// 7
print("옛 형식으로 끊긴 audio.m4a(목차 없음)는 broken, 파일은 그대로")
do {
  let dir = freshDir("legacy")
  try! FileManager.default.removeItem(at: dir.appendingPathComponent("parts"))
  let audio = dir.appendingPathComponent("audio.m4a")
  var bytes = Data([0, 0, 0, 0x1c]) + Data("ftypM4A ".utf8)
  bytes.append(Data(count: 64_000))
  try! bytes.write(to: audio)
  let (outcome, _) = try! RecordingFiles.finalize(audio: audio)
  check(outcome == .broken, "결과 broken")
  check(exists(audio), "파일을 지우지 않는다")
}

// 8
print("구간과 함께 audio.m4a가 이미 있으면 지우지 않고 옆에 옮겨 둔다")
do {
  let dir = freshDir("existing")
  let audio = dir.appendingPathComponent("audio.m4a")
  try! Data("old".utf8).write(to: audio)
  writeWav(dir.appendingPathComponent("parts/0001.wav"), seconds: 1)
  let (outcome, _) = try! RecordingFiles.finalize(audio: audio)
  let kept = (try! FileManager.default.contentsOfDirectory(atPath: dir.path)).filter { $0.hasPrefix("previous-") }
  check(outcome == .done, "결과 done")
  check(kept.count == 1, "이전 파일을 previous-*.m4a로 남긴다 (\(kept))")
}

// 9
print("합치다 실패하면 오류를 내고 구간을 남긴다")
do {
  let dir = freshDir("fail")
  let part = dir.appendingPathComponent("parts/0001.wav")
  writeWav(part, seconds: 1)
  // 녹음 폴더에 쓸 수 없게 해 임시 m4a를 못 만들게 한다
  try! FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path)
  var threw = false
  do { _ = try RecordingFiles.finalize(audio: dir.appendingPathComponent("audio.m4a")) } catch { threw = true }
  try! FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir.path)
  check(threw, "오류를 낸다")
  check(near(WavFile.duration(of: part) ?? 0, 1), "구간 파일이 그대로 있다")
  let (outcome, seconds) = try! RecordingFiles.finalize(audio: dir.appendingPathComponent("audio.m4a"))
  check(outcome == .done && near(seconds, 1), "다시 하면 합쳐진다 (\(outcome), \(seconds))")
}

print(failures == 0 ? "\n모두 통과" : "\n실패 \(failures)개")
exit(failures == 0 ? 0 : 1)
