// 녹음을 시작한 곳. 위치를 한 번 받고(requestLocation) Apple 지오코더로 주소로 바꾼다.
// 권한은 "앱을 사용하는 동안". 처음 묻는 것은 앱이 화면에 있을 때만 한다(동작 버튼으로 백그라운드에서 시작하면 건너뛴다)
// @see docs/organize.md

import CoreLocation
import UIKit

/// 위치 하나를 받으면 끝난다. 메인 스레드에서 쓴다
final class LocationFetcher: NSObject, CLLocationManagerDelegate {
  private let manager = CLLocationManager()
  private var completion: ((CLLocation?) -> Void)?
  private var timer: Timer?
  private var requested = false

  func fetch(timeout: TimeInterval, completion: @escaping (CLLocation?) -> Void) {
    self.completion = completion
    timer = Timer.scheduledTimer(withTimeInterval: timeout, repeats: false) { [weak self] _ in self?.finish(nil) }
    manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
    manager.delegate = self
    handle(manager.authorizationStatus)
  }

  private func handle(_ status: CLAuthorizationStatus) {
    switch status {
    case .authorizedWhenInUse, .authorizedAlways:
      if !requested {
        requested = true
        manager.requestLocation()
      }
    case .notDetermined:
      if UIApplication.shared.applicationState == .active {
        manager.requestWhenInUseAuthorization()
      } else {
        finish(nil)
      }
    default:
      finish(nil)
    }
  }

  func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
    handle(manager.authorizationStatus)
  }

  func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
    finish(locations.last)
  }

  func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
    finish(nil)
  }

  private func finish(_ location: CLLocation?) {
    guard let completion else { return }
    self.completion = nil
    timer?.invalidate()
    timer = nil
    completion(location)
  }
}

/// 주소 조각에서 겹치는 것을 빼고 잇는다. "서울특별시 중구 세종대로 110"
func describePlace(_ p: CLPlacemark?) -> String? {
  guard let p else { return nil }
  var out: [String] = []
  for part in [p.administrativeArea, p.locality, p.subLocality, p.name].compactMap({ $0?.trimmingCharacters(in: .whitespaces) }) where !part.isEmpty {
    if !out.contains(where: { $0 == part || $0.contains(part) }) { out.append(part) }
  }
  return out.isEmpty ? nil : out.joined(separator: " ")
}
