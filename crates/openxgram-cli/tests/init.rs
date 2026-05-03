//! xgram init — 비대화 모드 통합 테스트.
//!
//! 환경변수 `XGRAM_KEYSTORE_PASSWORD` 와 (옵션) `XGRAM_SEED` 를 통해
//! end-to-end 흐름 (Step 1~6 + manifest 작성·서명) 을 검증한다.
//!
//! 테스트는 환경변수를 공유하므로 `#[serial_test]` 없이는 단일 스레드(`--test-threads=1`)
//! 권장 — 본 파일은 그 가정 없이 명시적으로 unsafe `set_var` 호출 시 직전·직후로 한정한다.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::{InstallManifest, MachineRole};
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "test-machine".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_password() {
    // SAFETY: 단일 스레드 테스트에서만 호출. cargo test --test init 은 파일별 binary 라
    // 다른 통합 테스트 파일과 환경 격리됨.
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
    }
}

fn unset_seed() {
    unsafe {
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn init_creates_full_install_layout_and_signs_manifest() {
    set_password();
    unset_seed();
    let tmp = tempdir().unwrap();

    run_init(&opts(tmp.path().to_path_buf())).unwrap();

    // 디렉토리 존재
    assert!(tmp.path().join("keystore").is_dir());
    assert!(tmp.path().join("backup").is_dir());

    // master 키
    let ks = FsKeystore::new(tmp.path().join("keystore"));
    let entries = ks.list().unwrap();
    assert_eq!(entries.len(), 1, "master 키 1개만 존재");
    assert_eq!(entries[0].name, "master");
    assert_eq!(entries[0].derivation_path, "m/44'/60'/0'/0/0");

    // DB 파일
    assert!(tmp.path().join("db.sqlite").exists());

    // manifest + 서명 검증
    let manifest_path = tmp.path().join("install-manifest.json");
    let manifest = InstallManifest::read(&manifest_path).unwrap();
    assert_eq!(manifest.machine.alias, "test-machine");
    assert_eq!(manifest.registered_keys.len(), 1);
    assert!(!manifest.uninstall_token.is_empty(), "서명 채워짐");

    let kp = ks.load("master", TEST_PASSWORD).unwrap();
    manifest
        .verify_signature(&kp.public_key_bytes(), &manifest.uninstall_token)
        .expect("서명 검증 통과");
}

#[test]
fn init_dry_run_makes_no_changes() {
    set_password();
    unset_seed();
    let tmp = tempdir().unwrap();
    let mut o = opts(tmp.path().to_path_buf());
    o.dry_run = true;

    run_init(&o).unwrap();

    // 어떤 파일·디렉토리도 생성되지 않아야 함 (tmpdir 자체는 외부에서 만든 것)
    assert!(!tmp.path().join("keystore").exists());
    assert!(!tmp.path().join("backup").exists());
    assert!(!tmp.path().join("db.sqlite").exists());
    assert!(!tmp.path().join("install-manifest.json").exists());
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    set_password();
    unset_seed();
    let tmp = tempdir().unwrap();

    // 1차 설치 성공
    run_init(&opts(tmp.path().to_path_buf())).unwrap();

    // 2차 설치 — force=false → raise
    let err = run_init(&opts(tmp.path().to_path_buf())).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("이미 설치"), "msg={msg}");
}

#[test]
fn init_short_password_raises() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "short");
    }
    unset_seed();
    let tmp = tempdir().unwrap();

    let err = run_init(&opts(tmp.path().to_path_buf())).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("최소 12자"), "msg={msg}");

    // restore 정상 패스워드 (다른 테스트 영향 차단)
    set_password();
}

#[test]
fn init_import_requires_seed_env() {
    set_password();
    unset_seed();
    let tmp = tempdir().unwrap();
    let mut o = opts(tmp.path().to_path_buf());
    o.import = true;

    let err = run_init(&o).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("XGRAM_SEED"), "msg={msg}");
}
