//! 녹음을 시작한 곳. macOS는 Wi-Fi로 위치를 한 번 받아 Apple 지오코더로 주소로 바꾼다. 다른 OS는 남기지 않는다.
//! 위치 서비스가 꺼졌거나 허락하지 않았으면 조용히 건너뛴다. 녹음은 그대로 된다.
//! @see docs/organize.md

use crate::store::Location;

/// 위치를 기다리는 최대 시간. 처음에는 macOS가 허락을 묻는 동안 끝날 수 있다(다음 녹음부터 남는다)
#[cfg(target_os = "macos")]
const FIX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(target_os = "macos")]
const GEOCODE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// 지금 위치와 주소. 오래 걸리므로(최대 약 18초) 작업 스레드에서 부른다
#[cfg(target_os = "macos")]
pub fn capture() -> Option<Location> {
    let (lat, lon, accuracy) = mac::fix(FIX_TIMEOUT)?;
    Some(Location {
        lat,
        lon,
        accuracy: Some(accuracy),
        place: mac::place(lat, lon, GEOCODE_TIMEOUT),
        captured_at: chrono::Utc::now(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn capture() -> Option<Location> {
    None
}

/// 주소 조각에서 겹치는 것을 빼고 잇는다. "서울특별시", "서울특별시", "중구", "세종대로 110" → "서울특별시 중구 세종대로 110"
pub fn join_place(parts: &[Option<String>]) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for p in parts.iter().flatten().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if !out.iter().any(|o| o == p || o.contains(p)) {
            out.push(p.to_string());
        }
    }
    (!out.is_empty()).then(|| out.join(" "))
}

// CLGeocoder, CLPlacemark는 macOS 26에서 MapKit(MKReverseGeocodingRequest)으로 대체 표시됐지만, 그 API는 26부터라
// 최소 지원 버전(14.2)에서 쓸 수 없다. 동작은 그대로라 경고만 끈다
#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod mac {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::AllocAnyThread;
    use objc2_core_location::{kCLLocationAccuracyHundredMeters, CLAuthorizationStatus, CLGeocoder, CLLocation, CLLocationManager, CLPlacemark};
    use objc2_foundation::{NSArray, NSDate, NSError, NSRunLoop};

    fn pump(run_loop: &NSRunLoop) {
        run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.25));
    }

    /// 위도, 경도, 오차(m). 이 스레드의 런루프를 돌리며 기다린다(CLLocationManager는 만든 스레드의 런루프로 받는다)
    pub fn fix(timeout: Duration) -> Option<(f64, f64, f64)> {
        unsafe {
            if !CLLocationManager::locationServicesEnabled_class() {
                return None;
            }
            let manager = CLLocationManager::new();
            let status = manager.authorizationStatus();
            if status == CLAuthorizationStatus::Denied || status == CLAuthorizationStatus::Restricted {
                return None;
            }
            manager.setDesiredAccuracy(kCLLocationAccuracyHundredMeters);
            if status == CLAuthorizationStatus::NotDetermined {
                manager.requestWhenInUseAuthorization();
            }
            manager.startUpdatingLocation();
            let run_loop = NSRunLoop::currentRunLoop();
            let started = Instant::now();
            let mut best: Option<(f64, f64, f64)> = None;
            while started.elapsed() < timeout {
                pump(&run_loop);
                let status = manager.authorizationStatus();
                if status == CLAuthorizationStatus::Denied || status == CLAuthorizationStatus::Restricted {
                    break;
                }
                let Some(loc) = manager.location() else { continue };
                let accuracy = loc.horizontalAccuracy();
                // 노트북을 옮긴 뒤 예전 장소가 찍히지 않도록 2분 넘은 위치는 쓰지 않는다
                let age = -loc.timestamp().timeIntervalSinceNow();
                if accuracy <= 0.0 || age > 120.0 {
                    continue;
                }
                let c = loc.coordinate();
                if best.is_none_or(|(_, _, a)| accuracy < a) {
                    best = Some((c.latitude, c.longitude, accuracy));
                }
                if accuracy <= 100.0 {
                    break;
                }
            }
            manager.stopUpdatingLocation();
            best
        }
    }

    fn text(s: Option<objc2::rc::Retained<objc2_foundation::NSString>>) -> Option<String> {
        s.map(|s| s.to_string())
    }

    fn describe(p: &CLPlacemark) -> Option<String> {
        unsafe {
            super::join_place(&[
                text(p.administrativeArea()),
                text(p.locality()),
                text(p.subLocality()),
                text(p.name()),
            ])
        }
    }

    pub fn place(lat: f64, lon: f64, timeout: Duration) -> Option<String> {
        unsafe {
            let location = CLLocation::initWithLatitude_longitude(CLLocation::alloc(), lat, lon);
            let geocoder = CLGeocoder::new();
            let (tx, rx) = mpsc::channel::<Option<String>>();
            let block = RcBlock::new(move |marks: *mut NSArray<CLPlacemark>, _error: *mut NSError| {
                let found = marks.as_ref().and_then(|a| a.firstObject()).and_then(|p| describe(&p));
                let _ = tx.send(found);
            });
            geocoder.reverseGeocodeLocation_completionHandler(&location, RcBlock::as_ptr(&block));
            // 결과는 메인 스레드로 오는데, 이 스레드에서 부른 경우를 대비해 런루프도 돌린다
            let run_loop = NSRunLoop::currentRunLoop();
            let started = Instant::now();
            while started.elapsed() < timeout {
                if let Ok(found) = rx.try_recv() {
                    return found;
                }
                pump(&run_loop);
            }
            geocoder.cancelGeocode();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 겹치는_주소_조각은_한_번만_쓴다() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            join_place(&[s("서울특별시"), s("서울특별시"), s("중구"), s("세종대로 110")]).as_deref(),
            Some("서울특별시 중구 세종대로 110")
        );
        assert_eq!(join_place(&[None, s(" ")]), None);
    }
}
