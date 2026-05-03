// OpenXgram Desktop — Tauri 2.x 진입점.
//
// 현재 baseline:
//   - 단일 윈도우 (900×600)
//   - Tauri 명령 1개: get_status — `xgram doctor --json` 실행 결과 반환
//
// 후속:
//   - Sessions 탭 (xgram session list/recall)
//   - Vault 탭 (vault list + pending approve)
//   - 다국어 (ko/en)
//   - 자동 업데이트 (tauri updater)

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::process::Command;

use serde::Serialize;

#[derive(Serialize)]
struct StatusResult {
    success: bool,
    output: String,
}

#[tauri::command]
fn get_status() -> StatusResult {
    // 사용자 환경의 xgram 바이너리 호출 (PATH 가정).
    // 실패해도 패닉 없이 success=false 로 직렬화.
    match Command::new("xgram").args(["doctor", "--json"]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: String::from_utf8_lossy(&out.stdout).into_owned(),
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}\n\n`xgram` 이 PATH 에 있는지 확인."),
        },
    }
}

#[tauri::command]
fn get_version() -> StatusResult {
    match Command::new("xgram").args(["version", "--json"]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: String::from_utf8_lossy(&out.stdout).into_owned(),
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}"),
        },
    }
}

/// `xgram dump <kind>` 결과 JSON 그대로 반환. kind: sessions/vault/pending/peers/...
#[tauri::command]
fn dump(kind: String) -> StatusResult {
    match Command::new("xgram").args(["dump", &kind]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: if out.status.success() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                String::from_utf8_lossy(&out.stderr).into_owned()
            },
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}"),
        },
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_status, get_version, dump])
        .run(tauri::generate_context!())
        .expect("OpenXgram desktop 실행 실패");
}
