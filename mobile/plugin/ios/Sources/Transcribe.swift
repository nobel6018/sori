// iPhone에서 바로 미리 전사(iOS 26 SpeechAnalyzer, 기기 안에서 무료). 화자는 나누지 않는다.
// Mac이 전사하기 전에 내용을 대략 보고 찾으려는 것이다. Mac의 전사와 화자 분리는 그대로 따로 한다.
// @see docs/organize.md

import AVFoundation
import Foundation
import Speech

struct PreviewUtterance {
  let start: Double
  let end: Double
  let text: String
}

enum PreviewError: LocalizedError {
  case unsupportedLocale
  case noAsset

  var errorDescription: String? {
    switch self {
    case .unsupportedLocale: return "이 기기 언어는 iPhone 미리 전사를 지원하지 않습니다"
    case .noAsset: return "미리 전사 언어 모델을 받지 못했습니다"
    }
  }
}

/// 기기 언어로 전사한다. 지원하지 않으면 한국어. 언어 모델이 없으면 처음 한 번 받는다
@available(iOS 26.0, *)
func runPreviewTranscription(path: String) async throws -> (utterances: [PreviewUtterance], duration: Double) {
  let wanted = await SpeechTranscriber.supportedLocale(equivalentTo: Locale.current)
  let fallback = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko_KR"))
  guard let locale = wanted ?? fallback else { throw PreviewError.unsupportedLocale }
  let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
  if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
    try await request.downloadAndInstall()
  }
  if await AssetInventory.status(forModules: [transcriber]) != .installed {
    throw PreviewError.noAsset
  }

  let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
  let duration = Double(file.length) / file.processingFormat.sampleRate
  let analyzer = SpeechAnalyzer(modules: [transcriber])
  let collect = Task { () -> [PreviewUtterance] in
    var out: [PreviewUtterance] = []
    for try await result in transcriber.results where result.isFinal {
      let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
      if text.isEmpty { continue }
      out.append(PreviewUtterance(start: result.range.start.seconds, end: result.range.end.seconds, text: text))
    }
    return out
  }
  if let last = try await analyzer.analyzeSequence(from: file) {
    try await analyzer.finalizeAndFinish(through: last)
  } else {
    await analyzer.cancelAndFinishNow()
  }
  return (try await collect.value, duration)
}
