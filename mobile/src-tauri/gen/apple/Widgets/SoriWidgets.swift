// sori 위젯 확장. 녹음 중 Live Activity(잠금 화면, Dynamic Island)를 그린다.
// @see docs/mobile.md

import ActivityKit
import AppIntents
import SwiftUI
import WidgetKit

@main
struct SoriWidgets: WidgetBundle {
  var body: some Widget {
    RecordingLiveActivity()
  }
}

struct RecordingLiveActivity: Widget {
  var body: some WidgetConfiguration {
    ActivityConfiguration(for: RecordingActivityAttributes.self) { context in
      HStack(spacing: 12) {
        Image(systemName: "waveform")
          .font(.title2)
          .foregroundStyle(.red)
        VStack(alignment: .leading, spacing: 2) {
          Text("sori 녹음 중").font(.headline)
          Text("화면을 꺼도 이어집니다").font(.caption).foregroundStyle(.secondary)
        }
        Spacer()
        Elapsed(since: context.state.startedAt)
          .font(.title2.monospacedDigit())
        StopButton()
      }
      .padding()
    } dynamicIsland: { context in
      DynamicIsland {
        DynamicIslandExpandedRegion(.leading) {
          Label("녹음 중", systemImage: "waveform")
            .foregroundStyle(.red)
            .padding(.leading, 4)
        }
        DynamicIslandExpandedRegion(.trailing) {
          Elapsed(since: context.state.startedAt)
            .font(.title3.monospacedDigit())
            .padding(.trailing, 4)
        }
        DynamicIslandExpandedRegion(.bottom) {
          StopButton()
        }
      } compactLeading: {
        Image(systemName: "waveform").foregroundStyle(.red)
      } compactTrailing: {
        Elapsed(since: context.state.startedAt)
          .monospacedDigit()
          .frame(maxWidth: 52)
      } minimal: {
        Image(systemName: "mic.fill").foregroundStyle(.red)
      }
    }
  }
}

/// 시작 시각부터 위로 세는 시계. 시스템이 직접 세므로 앱이 갱신을 보내지 않아도 된다
private struct Elapsed: View {
  let since: Date

  var body: some View {
    Text(timerInterval: since...Date.distantFuture, countsDown: false)
      .multilineTextAlignment(.trailing)
  }
}

private struct StopButton: View {
  var body: some View {
    Button(intent: StopRecordingIntent()) {
      Image(systemName: "stop.fill")
        .font(.title3)
        .foregroundStyle(.white)
        .frame(width: 44, height: 44)
        .background(Circle().fill(.red))
    }
    .buttonStyle(.plain)
    .accessibilityLabel("녹음 멈추기")
  }
}
