//! 위치 받기 진단. 앱과 같은 방식(작업 스레드, 런루프 폴링)으로 상태를 0.5초마다 출력한다
//! cargo run --example location_check
#[cfg(target_os = "macos")]
fn main() {
    use objc2_core_location::{kCLLocationAccuracyHundredMeters, CLLocationManager};
    use objc2_foundation::{NSDate, NSRunLoop};
    let on_thread = std::env::args().nth(1).as_deref() != Some("main");
    let run = move || unsafe {
        println!("enabled={}", CLLocationManager::locationServicesEnabled_class());
        let m = CLLocationManager::new();
        println!("status={:?}", m.authorizationStatus());
        m.setDesiredAccuracy(kCLLocationAccuracyHundredMeters);
        m.requestWhenInUseAuthorization();
        m.startUpdatingLocation();
        let rl = NSRunLoop::currentRunLoop();
        let t = std::time::Instant::now();
        while t.elapsed().as_secs() < 15 {
            let before = std::time::Instant::now();
            rl.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.5));
            let spent = before.elapsed().as_millis();
            let loc = m.location().map(|l| (l.horizontalAccuracy(), -l.timestamp().timeIntervalSinceNow()));
            println!("{:>5}ms pump={spent}ms status={:?} loc={loc:?}", t.elapsed().as_millis(), m.authorizationStatus());
            std::thread::sleep(std::time::Duration::from_millis(if spent < 100 { 500 } else { 0 }));
        }
    };
    if on_thread { std::thread::spawn(run).join().unwrap() } else { run() }
}
#[cfg(not(target_os = "macos"))]
fn main() {}
