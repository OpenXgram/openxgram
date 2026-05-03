//! openxgram-manifest — install-manifest 타입·영속화·검증
//!
//! SPEC-lifecycle §4 의 단일 source of truth.
//! 디스크 read/write, secp256k1 ECDSA 서명 검증, drift 감지 (§4.3).

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallManifest {
    pub version: String,
    pub installed_at: DateTime<FixedOffset>,
    pub machine: Machine,
    pub uninstall_token: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub directories: Vec<DirectoryEntry>,
    #[serde(default)]
    pub system_services: Vec<SystemService>,
    #[serde(default)]
    pub binaries: Vec<BinaryEntry>,
    #[serde(default)]
    pub shell_integrations: Vec<ShellIntegration>,
    #[serde(default)]
    pub external_resources: Vec<ExternalResource>,
    #[serde(default)]
    pub registered_keys: Vec<RegisteredKey>,
    #[serde(default)]
    pub ports: Vec<PortEntry>,
    #[serde(default)]
    pub os_keychain_entries: Vec<KeychainEntry>,
    #[serde(default)]
    pub selected_extractors: serde_json::Value,
    #[serde(default)]
    pub inbound_webhook_port: Option<u16>,
    #[serde(default)]
    pub backup_schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Machine {
    pub alias: String,
    pub role: MachineRole,
    pub os: OsKind,
    pub arch: String,
    pub hostname: String,
    pub tailscale_ip: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MachineRole {
    Primary,
    Secondary,
    Worker,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OsKind {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
    pub installed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub created_by_installer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemService {
    pub name: String,
    #[serde(rename = "type")]
    pub service_type: ServiceType,
    pub unit_file: PathBuf,
    pub enabled: bool,
    pub started: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceType {
    SystemdUser,
    SystemdSystem,
    LaunchdUser,
    LaunchdSystem,
    WindowsService,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryEntry {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellIntegration {
    pub path: PathBuf,
    pub marker_start: String,
    pub marker_end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredKey {
    pub alias: String,
    pub address: String,
    pub derivation_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortEntry {
    pub number: u16,
    pub protocol: PortProtocol,
    pub service: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeychainEntry {
    pub service: String,
    pub account: String,
}

/// drift 감지 결과 — SPEC §4.3
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftItem {
    /// SHA256 불일치 (file·binary)
    Drift {
        kind: &'static str,
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// 경로·셸 마커·서비스 파일 누락
    Missing {
        kind: &'static str,
        path: PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("unsupported manifest version: found {found}, expected {expected}")]
    UnsupportedVersion { found: String, expected: String },

    #[error("invalid signature encoding: {0}")]
    InvalidSignatureEncoding(String),

    #[error("invalid public key encoding")]
    InvalidPublicKey,

    #[error("signature verification failed")]
    SignatureVerification,
}

pub type Result<T> = std::result::Result<T, ManifestError>;

impl InstallManifest {
    /// 디스크에서 읽고 schema version 검증
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref())?;
        let manifest: Self = serde_json::from_slice(&bytes)?;
        if manifest.version != SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                found: manifest.version,
                expected: SCHEMA_VERSION.into(),
            });
        }
        Ok(manifest)
    }

    /// atomic 쓰기: 같은 디렉토리에 .tmp 작성 후 rename
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let target = path.as_ref();
        let mut tmp = target.to_path_buf();
        tmp.set_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, target)?;
        Ok(())
    }

    /// 서명 대상 정규화 — uninstall_token 필드를 공란으로 두고 직렬화.
    /// struct 필드 순서가 안정적이므로 동일 manifest는 동일 바이트.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut clone = self.clone();
        clone.uninstall_token.clear();
        Ok(serde_json::to_vec(&clone)?)
    }

    /// secp256k1 ECDSA 서명 검증.
    /// public_key_compressed: 33바이트 압축 공개키. signature_hex: r|s 64바이트 hex.
    pub fn verify_signature(
        &self,
        public_key_compressed: &[u8],
        signature_hex: &str,
    ) -> Result<()> {
        use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

        let sig_bytes = hex::decode(signature_hex)
            .map_err(|e| ManifestError::InvalidSignatureEncoding(e.to_string()))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|e| ManifestError::InvalidSignatureEncoding(e.to_string()))?;
        let vk = VerifyingKey::from_sec1_bytes(public_key_compressed)
            .map_err(|_| ManifestError::InvalidPublicKey)?;

        let canonical = self.canonical_bytes()?;
        vk.verify(&canonical, &sig)
            .map_err(|_| ManifestError::SignatureVerification)
    }
}

/// SPEC §4.3 drift 감지.
///
/// 다음 카테고리만 manifest crate가 직접 처리한다:
/// - files: SHA256 불일치 → Drift, 누락 → Missing
/// - directories: 경로 누락 → Missing
/// - binaries: SHA256 불일치 → Drift, 누락 → Missing
/// - system_services: unit_file 누락 → Missing (enabled/started 상태는 OS 매니저 호출 필요 → lifecycle 레이어)
/// - shell_integrations: 마커 블록 누락 → Missing
///
/// ports CONFLICT 검사는 OS별 프로세스 조회가 필요해 lifecycle 레이어가 담당.
pub fn detect_drift(manifest: &InstallManifest) -> Vec<DriftItem> {
    let mut items = Vec::new();

    for f in &manifest.files {
        check_hash(&mut items, "file", &f.path, &f.sha256);
    }

    for d in &manifest.directories {
        if !d.path.exists() {
            items.push(DriftItem::Missing {
                kind: "directory",
                path: d.path.clone(),
            });
        }
    }

    for b in &manifest.binaries {
        check_hash(&mut items, "binary", &b.path, &b.sha256);
    }

    for s in &manifest.system_services {
        if !s.unit_file.exists() {
            items.push(DriftItem::Missing {
                kind: "service",
                path: s.unit_file.clone(),
            });
        }
    }

    for sh in &manifest.shell_integrations {
        match std::fs::read_to_string(&sh.path) {
            Ok(contents)
                if !contents.contains(&sh.marker_start)
                    || !contents.contains(&sh.marker_end) =>
            {
                items.push(DriftItem::Missing {
                    kind: "shell_marker",
                    path: sh.path.clone(),
                });
            }
            Err(_) => items.push(DriftItem::Missing {
                kind: "shell_file",
                path: sh.path.clone(),
            }),
            _ => {}
        }
    }

    items
}

fn check_hash(items: &mut Vec<DriftItem>, kind: &'static str, path: &Path, expected: &str) {
    match sha256_of(path) {
        Ok(actual) if actual != expected => items.push(DriftItem::Drift {
            kind,
            path: path.to_path_buf(),
            expected: expected.to_string(),
            actual,
        }),
        Err(_) => items.push(DriftItem::Missing {
            kind,
            path: path.to_path_buf(),
        }),
        _ => {}
    }
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

// Display impls — serde rename_all 과 동일한 표기. 사용자 출력·로그에 사용.
impl std::fmt::Display for MachineRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Worker => "worker",
        })
    }
}

impl std::fmt::Display for OsKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        })
    }
}

impl std::fmt::Display for ServiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SystemdUser => "systemd-user",
            Self::SystemdSystem => "systemd-system",
            Self::LaunchdUser => "launchd-user",
            Self::LaunchdSystem => "launchd-system",
            Self::WindowsService => "windows-service",
        })
    }
}

impl std::fmt::Display for PortProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        })
    }
}
