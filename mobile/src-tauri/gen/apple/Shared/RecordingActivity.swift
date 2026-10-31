// 녹음 중에 잠금 화면과 Dynamic Island에 띄우는 Live Activity의 데이터.
// 앱 타깃과 위젯 확장(Widgets/)이 함께 컴파일한다.
// @see docs/mobile.md

import ActivityKit
import Foundation

struct RecordingActivityAttributes: ActivityAttributes {
  struct ContentState: Codable, Hashable {
    /// 경과 시간의 기준. 화면이 스스로 시간을 세므로 녹음 중에 갱신을 보내지 않는다
    var startedAt: Date
  }
}
