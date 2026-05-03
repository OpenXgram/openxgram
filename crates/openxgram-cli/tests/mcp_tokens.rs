//! MCP HTTP Bearer 토큰 발급·검증·폐기 통합 테스트.

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::mcp_tokens;
use openxgram_manifest::MachineRole;
use std::path::PathBuf;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "mcp-tok-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "test-password-12345");
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn create_then_verify_round_trip() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut db = mcp_tokens::open_db(&data_dir).unwrap();
    let (id, plain) = mcp_tokens::create_token(&mut db, "0xAlice", Some("laptop")).unwrap();
    assert_eq!(plain.len(), 64); // 32 bytes hex
    assert!(!id.is_empty());

    let agent = mcp_tokens::verify_token(&mut db, &plain).unwrap();
    assert_eq!(agent.as_deref(), Some("0xAlice"));
}

#[test]
fn verify_unknown_token_returns_none() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut db = mcp_tokens::open_db(&data_dir).unwrap();
    let agent = mcp_tokens::verify_token(&mut db, "deadbeef").unwrap();
    assert!(agent.is_none());
}

#[test]
fn list_then_revoke() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut db = mcp_tokens::open_db(&data_dir).unwrap();
    let (id1, _) = mcp_tokens::create_token(&mut db, "0xAlice", None).unwrap();
    let (id2, _) = mcp_tokens::create_token(&mut db, "0xBob", Some("phone")).unwrap();

    let entries = mcp_tokens::list_tokens(&mut db).unwrap();
    assert_eq!(entries.len(), 2);

    mcp_tokens::revoke_token(&mut db, &id1).unwrap();
    let after = mcp_tokens::list_tokens(&mut db).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, id2);
}

#[test]
fn revoke_unknown_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut db = mcp_tokens::open_db(&data_dir).unwrap();
    let err = mcp_tokens::revoke_token(&mut db, "nope").unwrap_err();
    assert!(format!("{err:#}").contains("nope"));
}

#[test]
fn token_hash_changes_for_different_inputs() {
    let h1 = mcp_tokens::hash_token("a");
    let h2 = mcp_tokens::hash_token("b");
    assert_ne!(h1, h2);
    assert_eq!(h1.len(), 64); // sha256 hex
}

#[test]
fn generate_token_returns_64_hex_chars() {
    let t = mcp_tokens::generate_token();
    assert_eq!(t.len(), 64);
    assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
}
