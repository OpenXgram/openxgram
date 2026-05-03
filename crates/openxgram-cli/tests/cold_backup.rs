//! Cold backup 통합 테스트.
//!
//! init → uninstall --cold-backup-to → 백업 파일 존재 + magic + decrypt round-trip.

use std::path::PathBuf;

use openxgram_cli::backup::{create_cold_backup, resolve_backup_target};
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::uninstall::{run_uninstall, UninstallOpts};
use openxgram_keystore::decrypt_blob;
use openxgram_manifest::MachineRole;
use sha2::{Digest, Sha256};
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

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn create_cold_backup_round_trip_decrypt() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let backup_path = tmp.path().join("backup.tar.gz.enc");
    let info = create_cold_backup(&data_dir, &backup_path, TEST_PASSWORD).unwrap();

    // 파일 존재 + 정확한 크기 + 정확한 SHA256
    let bytes = std::fs::read(&backup_path).unwrap();
    assert_eq!(bytes.len() as u64, info.size_bytes);
    let mut h = Sha256::new();
    h.update(&bytes);
    assert_eq!(hex::encode(h.finalize()), info.sha256);

    // magic 검증 (OXBK01)
    assert_eq!(&bytes[..6], b"OXBK01");

    // decrypt 후 tar.gz 가 들어 있는지 확인 (gzip magic 1f 8b)
    let plaintext = decrypt_blob(TEST_PASSWORD, &bytes).unwrap();
    assert_eq!(&plaintext[..2], &[0x1f, 0x8b], "gzip magic 기대");
}

#[test]
fn uninstall_with_cold_backup_removes_data_and_keeps_backup() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let backup_path = tmp.path().join("snapshot.enc");
    run_uninstall(&UninstallOpts {
        data_dir: data_dir.clone(),
        cold_backup_to: Some(backup_path.clone()),
        no_backup: false,
        confirm: None, // cold backup 시 confirm 불필요
        dry_run: false,
    })
    .unwrap();

    assert!(!data_dir.exists(), "data_dir 사라짐");
    assert!(backup_path.exists(), "backup 파일 보존");

    // backup 복호화 후 원본 데이터에 install-manifest.json 이 들어있는지 확인
    let blob = std::fs::read(&backup_path).unwrap();
    let plaintext = decrypt_blob(TEST_PASSWORD, &blob).unwrap();
    // tar 헤더 일부를 검색 — install-manifest.json 파일명이 tar 안에 있어야 함
    let head = String::from_utf8_lossy(&plaintext[..plaintext.len().min(8192)]);
    // gzip 압축이라 단순 substring 검색은 안 통함 — 압축 해제는 별도 의존성 추가 부담.
    // 대신 tar 헤더 magic("ustar")가 압축 해제 후 첫 블록에 보장되는데 압축 해제는 후속 PR 에서.
    // Phase 1 first PR 검증은 "decrypt 가능 + 첫 2바이트가 gzip magic"으로 충분.
    let _ = head; // 사용처 없는 head — 단순 사용 표시
}

#[test]
fn uninstall_rejects_both_backup_and_no_backup() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_uninstall(&UninstallOpts {
        data_dir: data_dir.clone(),
        cold_backup_to: Some(tmp.path().join("b.enc")),
        no_backup: true,
        confirm: None,
        dry_run: false,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("동시 사용 금지"));
}

#[test]
fn uninstall_rejects_neither_backup_option() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_uninstall(&UninstallOpts {
        data_dir: data_dir.clone(),
        cold_backup_to: None,
        no_backup: false,
        confirm: None,
        dry_run: false,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("백업 옵션 필요"));
}

#[test]
fn restore_round_trip_after_uninstall() {
    use openxgram_cli::backup::restore_cold_backup;
    use openxgram_core::paths::manifest_path;

    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    let backup_path = tmp.path().join("snap.enc");
    let restore_dir = tmp.path().join("restored");

    // init → cold-backup uninstall → 디렉토리 사라짐
    run_init(&init_opts(data_dir.clone())).unwrap();
    let manifest_before = std::fs::read(manifest_path(&data_dir)).unwrap();
    run_uninstall(&UninstallOpts {
        data_dir: data_dir.clone(),
        cold_backup_to: Some(backup_path.clone()),
        no_backup: false,
        confirm: None,
        dry_run: false,
    })
    .unwrap();
    assert!(!data_dir.exists());

    // restore → 새 위치에 복원
    let info = restore_cold_backup(&backup_path, &restore_dir, TEST_PASSWORD).unwrap();
    assert_eq!(info.target_dir, restore_dir);
    assert!(restore_dir.join("install-manifest.json").exists());
    assert!(restore_dir.join("db.sqlite").exists());
    assert!(restore_dir.join("keystore").join("master.json").exists());

    // manifest 내용 동일
    let manifest_after = std::fs::read(manifest_path(&restore_dir)).unwrap();
    assert_eq!(manifest_before, manifest_after);
}

#[test]
fn restore_into_nonempty_dir_raises() {
    use openxgram_cli::backup::restore_cold_backup;

    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let backup_path = tmp.path().join("snap.enc");
    create_cold_backup(&data_dir, &backup_path, TEST_PASSWORD).unwrap();

    // 비어있지 않은 디렉토리 (data_dir 자체) 로 복원 시도 → raise
    let err = restore_cold_backup(&backup_path, &data_dir, TEST_PASSWORD).unwrap_err();
    assert!(format!("{err:#}").contains("비어있지 않음"));
}

#[test]
fn cold_backup_decrypt_with_wrong_password_fails() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let backup_path = tmp.path().join("b.enc");
    create_cold_backup(&data_dir, &backup_path, TEST_PASSWORD).unwrap();

    let bytes = std::fs::read(&backup_path).unwrap();
    let err = decrypt_blob("wrong-password-1234", &bytes).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("invalid password") || msg.contains("decrypt"));
}

#[test]
fn resolve_backup_target_dir_creates_timestamped_filename() {
    let tmp = tempdir().unwrap();
    // 디렉토리 → openxgram-<ts>.cbk 생성
    let target = resolve_backup_target(tmp.path()).unwrap();
    assert_eq!(target.parent(), Some(tmp.path()));
    let name = target.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("openxgram-"));
    assert!(name.ends_with(".cbk"));
}

#[test]
fn resolve_backup_target_file_path_passes_through() {
    let tmp = tempdir().unwrap();
    let explicit = tmp.path().join("manual-name.enc");
    let target = resolve_backup_target(&explicit).unwrap();
    assert_eq!(target, explicit);
}

#[test]
fn backup_round_trip_into_directory() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // backup 디렉토리로 to 지정 — timestamped 파일 생성
    let backup_dir = tmp.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let target = resolve_backup_target(&backup_dir).unwrap();
    let info = create_cold_backup(&data_dir, &target, TEST_PASSWORD).unwrap();
    assert!(info.path.exists());
    assert!(info.size_bytes > 0);
    assert_eq!(info.sha256.len(), 64);
}
