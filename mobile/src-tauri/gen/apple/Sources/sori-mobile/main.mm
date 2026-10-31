#include "bindings/bindings.h"

// RecordingControl.swift. Rust가 녹음을 시작하고 멈출 때 Live Activity를 띄우고 내릴 함수를 넘긴다
extern "C" void sori_install_live_activity(void);

int main(int argc, char * argv[]) {
	sori_install_live_activity();
	ffi::start_app();
	return 0;
}
