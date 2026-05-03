//! openxgram-manifest — install-manifest 스키마 정의 및 검증
//!
//! xgram install 시 사용되는 manifest.json의 Rust 타입 표현과 검증 로직.
//! Phase 1: 타입 골격. 검증 로직은 Phase 2 이후.

use serde::{Deserialize, Serialize};

/// install-manifest.json 루트 구조
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallManifest {
    /// 매니페스트 스키마 버전
    pub manifest_version: String,

    /// 패키지 메타데이터
    pub package: PackageMeta,

    /// 설치 시 생성할 디렉토리 목록
    #[serde(default)]
    pub directories: Vec<DirectorySpec>,

    /// 설치 시 실행할 명령 (훅)
    #[serde(default)]
    pub hooks: Hooks,

    /// 의존 서비스 요구사항
    #[serde(default)]
    pub requirements: Requirements,
}

/// 패키지 메타데이터
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
}

/// 설치 디렉토리 명세
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySpec {
    /// 경로 (예: "~/.openxgram/data")
    pub path: String,
    /// 디렉토리 권한 (예: "700")
    pub mode: Option<String>,
}

/// 설치 훅 (Phase 2 구현 예정)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hooks {
    /// 설치 전 실행할 명령
    pub pre_install: Option<Vec<String>>,
    /// 설치 후 실행할 명령
    pub post_install: Option<Vec<String>>,
    /// 제거 전 실행할 명령
    pub pre_uninstall: Option<Vec<String>>,
}

/// 의존 서비스 요구사항
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Requirements {
    /// 최소 Rust 버전
    pub rust_version: Option<String>,
    /// 필요한 외부 바이너리 (예: ["tailscale"])
    pub binaries: Option<Vec<String>>,
    /// 최소 디스크 여유 공간 (MB)
    pub disk_mb: Option<u64>,
}

/// manifest 검증 에러
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("missing required field: {0}")]
    MissingField(String),

    #[error("invalid version format: {0}")]
    InvalidVersion(String),

    #[error("deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ManifestError>;

impl InstallManifest {
    /// JSON 문자열로부터 파싱
    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// 기본 검증 (Phase 2에서 JSON Schema 검증으로 강화 예정)
    pub fn validate(&self) -> Result<()> {
        if self.manifest_version.is_empty() {
            return Err(ManifestError::MissingField("manifest_version".into()));
        }
        if self.package.name.is_empty() {
            return Err(ManifestError::MissingField("package.name".into()));
        }
        if self.package.version.is_empty() {
            return Err(ManifestError::MissingField("package.version".into()));
        }
        Ok(())
    }
}
