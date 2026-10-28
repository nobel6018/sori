fn main() {
    // Tauri는 공용 컨트롤 v6 매니페스트를 앱 실행 파일에만 넣는다. 테스트 실행 파일에 이게 없으면
    // Windows가 시작 단계에서 "진입점을 찾을 수 없음" 오류 창을 띄우고, CI에서는 아무도 닫지 않아 멈춘다.
    // 기본 매니페스트를 끄고 링커로 넣어 앱, 테스트, 예제 실행 파일 모두에 들어가게 한다
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("tauri-build 실패");
    } else {
        tauri_build::build()
    }
}
