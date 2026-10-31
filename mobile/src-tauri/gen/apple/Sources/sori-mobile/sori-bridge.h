// Swift가 부르는 Rust 함수(mobile/src-tauri/src/intents.rs). 앱 타깃의 브리징 헤더다
#pragma once
#include <stdbool.h>

void sori_set_activity_hooks(const char *(*started)(void), void (*stopped)(void));
char *sori_intent_start_recording(void);
char *sori_intent_stop_recording(void);
bool sori_intent_is_recording(void);
void sori_intent_free(char *message);
