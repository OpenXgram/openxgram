//! xgram doctor — 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::doctor::{run_doctor, DoctorOpts, Verdict};
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "test-machine".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn doctor_opts(data_dir: PathBuf) -> DoctorOpts {
    DoctorOpts { data_dir }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn doctor_after_fresh_init_all_ok() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();
    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    report.print();

    let summary: String = report
        .checks
        .iter()
        .map(|c| format!("  {} {} — {}", c.verdict, c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(report.fail_count(), 0, "FAIL 0건 기대\n{summary}");
    // daemon 안 떠있으면 transport 항목 WARN — exit_code 1(FAIL) 만 아니면 OK
    assert_ne!(report.exit_code(), 1, "FAIL 없음 가정");
    assert!(report.ok_count() >= 4, "OK 4건 이상");
}

#[test]
fn doctor_without_install_returns_fail() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("absent");

    let report = run_doctor(&doctor_opts(data_dir)).unwrap();

    assert!(report.fail_count() >= 1, "manifest 미존재 → FAIL 최소 1건");
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn doctor_detects_corrupted_db() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();

    // DB 파일에 쓰레기 덮어쓰기 → integrity_check 실패
    std::fs::write(data_dir.join("db.sqlite"), b"this is not a sqlite file").unwrap();

    let report = run_doctor(&doctor_opts(data_dir)).unwrap();

    let db_check = report
        .checks
        .iter()
        .find(|c| c.name == "SQLite 무결성")
        .expect("SQLite 점검 결과 존재");
    assert_eq!(
        db_check.verdict,
        Verdict::Fail,
        "변조 DB → FAIL: {db_check:?}"
    );
    assert!(report.exit_code() >= 1);
}

#[test]
fn doctor_to_json_is_parseable() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let json = report.to_json().unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["checks"].is_array());
    assert!(v["summary"]["ok"].is_number());
    assert!(v["summary"]["warn"].is_number());
    assert!(v["summary"]["fail"].is_number());
}

#[cfg(unix)]
#[test]
fn doctor_warns_on_wrong_keystore_mode() {
    use std::os::unix::fs::PermissionsExt;

    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // 권한을 644 로 변경 → WARN
    let path = data_dir.join("keystore").join("master.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let ks_check = report
        .checks
        .iter()
        .find(|c| c.name == "Keystore master")
        .unwrap();
    assert_eq!(ks_check.verdict, Verdict::Warn);
}

#[test]
fn doctor_reports_memory_layer_counts() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let mem = report
        .checks
        .iter()
        .find(|c| c.name == "Memory layers")
        .expect("Memory layers check");
    assert_eq!(mem.verdict, Verdict::Ok);
    for table in ["messages", "episodes", "memories", "patterns", "traits"] {
        assert!(
            mem.detail.contains(table),
            "missing {table} in detail: {}",
            mem.detail
        );
    }
}

#[test]
fn doctor_reports_vault_layer_counts() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let v = report
        .checks
        .iter()
        .find(|c| c.name == "Vault layers")
        .expect("Vault layers check");
    assert_eq!(v.verdict, Verdict::Ok);
    assert!(v.detail.contains("entries=0"));
    assert!(v.detail.contains("acl=0"));
    assert!(v.detail.contains("audit=0"));
    assert!(v.detail.contains("denied_today=0"));
}

#[test]
fn doctor_includes_tailscale_check() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let ts = report
        .checks
        .iter()
        .find(|c| c.name == "Tailscale")
        .expect("Tailscale check");
    // 환경에 따라 Ok/Warn 둘 다 가능 — 검사가 패닉 없이 도는지만 확인
    assert!(matches!(ts.verdict, Verdict::Ok | Verdict::Warn));
}

#[test]
fn doctor_includes_embedder_mode_check() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let report = run_doctor(&doctor_opts(data_dir)).unwrap();
    let emb = report
        .checks
        .iter()
        .find(|c| c.name == "Embedder mode")
        .expect("Embedder mode check");
    // dummy 빌드 (기본) → WARN, fastembed 빌드 → OK
    assert!(matches!(emb.verdict, Verdict::Ok | Verdict::Warn));
    assert!(
        emb.detail.contains("Embedder")
            || emb.detail.contains("fastembed")
            || emb.detail.contains("Dummy")
    );
}
