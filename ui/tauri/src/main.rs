// OpenXgram Desktop — Tauri 2.x binary entry.
// 모든 invoke 핸들러는 lib.rs 에 있다.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    openxgram_desktop_lib::run();
}
