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
    // 잠금 화면은 한 줄로 작게 그린다. 녹음 중에는 iOS가 늘 띄우게 하므로 화면을 덜 가리게 한다
    ActivityConfiguration(for: RecordingActivityAttributes.self) { context in
      HStack(spacing: 10) {
        Image(systemName: "waveform")
          .font(.body)
          .foregroundStyle(.red)
        Text("sori 녹음 중").font(.subheadline)
        Spacer()
        Elapsed(since: context.state.startedAt)
          .font(.body.monospacedDigit())
        StopButton(size: 30)
      }
      .padding(.horizontal, 16)
      .padding(.vertical, 6)
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

/// 보이는 원은 size, 누르는 영역은 44pt를 지킨다
private struct StopButton: View {
  var size: CGFloat = 44

  var body: some View {
    Button(intent: StopRecordingIntent()) {
      Image(systemName: "stop.fill")
        .font(size < 44 ? .footnote : .title3)
        .foregroundStyle(.white)
        .frame(width: size, height: size)
        .background(Circle().fill(.red))
        .frame(width: 44, height: 44)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel("녹음 멈추기")
  }
}
