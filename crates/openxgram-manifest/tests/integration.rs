//! manifest crate 통합 테스트 — round-trip, version 검증, 서명, drift 감지.

use std::fs;

use chrono::{DateTime, FixedOffset};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::*;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn ts() -> DateTime<FixedOffset> {
    "2026-05-03T14:00:00+09:00".parse().unwrap()
}

fn sample(tmp: &std::path::Path) -> InstallManifest {
    InstallManifest {
        version: "1".into(),
        installed_at: ts(),
        machine: Machine {
            alias: "test".into(),
            role: MachineRole::Primary,
            os: OsKind::Linux,
            arch: "x86_64".into(),
            hostname: "h".into(),
            tailscale_ip: None,
        },
        uninstall_token: String::new(),
        files: vec![],
        directories: vec![DirectoryEntry {
            path: tmp.to_path_buf(),
            created_by_installer: false,
        }],
        system_services: vec![],
        binaries: vec![],
        shell_integrations: vec![],
        external_resources: vec![],
        registered_keys: vec![],
        ports: vec![],
        os_keychain_entries: vec![],
        selected_extractors: serde_json::Value::Null,
        inbound_webhook_port: Some(14921),
        backup_schedule: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[test]
fn round_trip_disk() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("install-manifest.json");
    let m = sample(tmp.path());

    m.write(&path).unwrap();
    let loaded = InstallManifest::read(&path).unwrap();
    assert_eq!(m, loaded);
}

#[test]
fn unsupported_version_raises() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("manifest.json");
    let mut m = sample(tmp.path());
    m.version = "999".into();
    m.write(&path).unwrap();

    let err = InstallManifest::read(&path).unwrap_err();
    assert!(matches!(err, ManifestError::UnsupportedVersion { .. }));
}

#[test]
fn signature_round_trip() {
    let tmp = tempdir().unwrap();
    let ks = FsKeystore::new(tmp.path().join("keystore"));
    let (_addr, _phrase) = ks.create("master", "pw").unwrap();
    let kp = ks.load("master", "pw").unwrap();

    let m = sample(tmp.path());
    let canonical = m.canonical_bytes().unwrap();
    let sig = kp.sign(&canonical);

    m.verify_signature(&kp.public_key_bytes(), &hex::encode(&sig))
        .unwrap();
}

#[test]
fn signature_tampering_raises() {
    let tmp = tempdir().unwrap();
    let ks = FsKeystore::new(tmp.path().join("keystore"));
    let _ = ks.create("master", "pw").unwrap();
    let kp = ks.load("master", "pw").unwrap();

    let m = sample(tmp.path());
    let sig = kp.sign(&m.canonical_bytes().unwrap());

    let mut tampered = m.clone();
    tampered.machine.hostname = "evil".into();

    let err = tampered
        .verify_signature(&kp.public_key_bytes(), &hex::encode(&sig))
        .unwrap_err();
    assert!(matches!(err, ManifestError::SignatureVerification));
}

#[test]
fn invalid_signature_encoding_raises() {
    let tmp = tempdir().unwrap();
    let ks = FsKeystore::new(tmp.path().join("keystore"));
    let _ = ks.create("master", "pw").unwrap();
    let kp = ks.load("master", "pw").unwrap();

    let m = sample(tmp.path());
    let err = m
        .verify_signature(&kp.public_key_bytes(), "not-hex-zz")
        .unwrap_err();
    assert!(matches!(err, ManifestError::InvalidSignatureEncoding(_)));
}

#[test]
fn detect_drift_files_dirs_and_drift() {
    let tmp = tempdir().unwrap();

    let real = tmp.path().join("real.txt");
    fs::write(&real, b"hello").unwrap();
    let correct = sha256_hex(b"hello");

    let mut m = sample(tmp.path());
    m.files = vec![
        FileEntry {
            path: real.clone(),
            sha256: correct,
            size_bytes: 5,
            installed_at: ts(),
        },
        FileEntry {
            path: tmp.path().join("missing.txt"),
            sha256: "0".repeat(64),
            size_bytes: 0,
            installed_at: ts(),
        },
        FileEntry {
            path: real.clone(),
            sha256: "f".repeat(64),
            size_bytes: 5,
            installed_at: ts(),
        },
    ];
    m.directories.push(DirectoryEntry {
        path: tmp.path().join("absent_dir"),
        created_by_installer: true,
    });

    let drift = detect_drift(&m);
    let missing = drift
        .iter()
        .filter(|d| matches!(d, DriftItem::Missing { .. }))
        .count();
    let drifted = drift
        .iter()
        .filter(|d| matches!(d, DriftItem::Drift { .. }))
        .count();
    assert_eq!(missing, 2, "missing.txt + absent_dir → Missing 2건");
    assert_eq!(drifted, 1, "real.txt with wrong hash → Drift 1건");
}

#[test]
fn detect_drift_shell_markers() {
    let tmp = tempdir().unwrap();
    let rc = tmp.path().join("bashrc");
    fs::write(&rc, "export PATH=/usr/bin\n# unrelated\n").unwrap();

    let mut m = sample(tmp.path());
    m.shell_integrations.push(ShellIntegration {
        path: rc.clone(),
        marker_start: "# BEGIN OPENXGRAM".into(),
        marker_end: "# END OPENXGRAM".into(),
    });
    m.shell_integrations.push(ShellIntegration {
        path: tmp.path().join("zshrc"),
        marker_start: "# BEGIN OPENXGRAM".into(),
        marker_end: "# END OPENXGRAM".into(),
    });

    let drift = detect_drift(&m);
    assert_eq!(drift.len(), 2);
    assert!(drift.iter().any(|d| matches!(
        d,
        DriftItem::Missing {
            kind: "shell_marker",
            ..
        }
    )));
    assert!(drift.iter().any(|d| matches!(
        d,
        DriftItem::Missing {
            kind: "shell_file",
            ..
        }
    )));
}
