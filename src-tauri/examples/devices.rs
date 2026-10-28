//! 오디오 장치와 시스템 소리 녹음 가능 여부를 출력한다
//! cargo run --example devices
use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    let name = |d: &cpal::Device| d.description().map(|d| d.name().to_string()).unwrap_or_default();
    if let Some(d) = host.default_input_device() {
        println!("기본 입력: {}", name(&d));
    }
    if let Some(d) = host.default_output_device() {
        let cfg = d.default_output_config().ok();
        println!(
            "기본 출력: {} (입력도 지원: {}, 설정: {:?})",
            name(&d),
            d.supports_input(),
            cfg.map(|c| (c.sample_rate(), c.channels()))
        );
    }
    for d in host.output_devices().into_iter().flatten() {
        println!("  출력 장치: {} (입력도 지원: {})", name(&d), d.supports_input());
    }
}
