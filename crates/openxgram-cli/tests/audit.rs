//! xgram audit — 통합 테스트 (PRD-AUDIT-03).

use std::path::PathBuf;

use openxgram_cli::audit::{run_audit, AuditAction, VerifyReport};
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-audit";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "audit-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
#[serial_test::file_serial]
fn audit_verify_on_clean_db_reports_ok() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();

    // 깨끗한 (audit row 가 없거나 minimal 한) DB 에서 verify → Ok
    let report = run_audit(&data_dir, AuditAction::Verify).unwrap();
    assert!(
        matches!(report, VerifyReport::Ok),
        "clean DB 에서 audit verify → Ok 기대, 실제: {report:?}"
    );

    // Display 출력에 "정상" 포함
    let s = format!("{report}");
    assert!(s.contains("정상"), "Display 메시지 'normal' 키워드 포함: {s}");
}
