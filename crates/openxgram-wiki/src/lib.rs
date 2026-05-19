//! openxgram-wiki — L2 위키 페이지 (Karpathy 패턴).
//!
//! 정본: docs/PRD-OpenXgram.md §4.1
//!
//! 디스크 markdown + DB 인덱스 동기화:
//!   - 디스크가 정본 (사용자가 직접 열람·수정 가능)
//!   - DB는 인덱스 (검색·벡터 KNN 회상용)
//!   - 동기화: write → 파일 + DB. watch → 파일 변경 → DB 갱신
//!   - 충돌: last-write-wins (사용자 수정 우선)
//!
//! 모듈:
//!   - page   : Page 도메인 + frontmatter 파싱
//!   - store  : DB 인덱스 (wiki_pages + wiki_embeddings)
//!   - fs     : 디스크 I/O ({XGRAM_DATA_DIR}/wiki/)
//!   - sync   : 양방향 동기화 + content_hash 검증
//!   - search : 벡터 + LIKE 검색
//!   - mcp    : 5개 MCP 도구 핸들러 (read/write/link/search/list)

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fs;
pub mod mcp;
pub mod page;
pub mod search;
pub mod store;
pub mod sync;

pub use fs::WikiFs;
pub use page::{Page, PageId, PageType, Related};
pub use search::{SearchHit, search_wiki};
pub use store::{WikiStore, WikiStoreError};
pub use sync::{Syncer, SyncReport};

use thiserror::Error;

/// 위키 모듈 전반 에러.
#[derive(Debug, Error)]
pub enum WikiError {
    /// DB 인덱스 에러.
    #[error("store error: {0}")]
    Store(WikiStoreError),

    /// 파일 시스템 에러.
    #[error("fs error: {0}")]
    Fs(#[from] std::io::Error),

    /// frontmatter 파싱 실패.
    #[error("frontmatter parse error: {0}")]
    Frontmatter(String),

    /// content_hash mismatch (사용자가 동시에 수정).
    /// 절대 규칙 1 (fallback 금지) — silent overwrite 금지.
    #[error("content_hash mismatch for page {id}: expected={expected}, actual={actual}")]
    ContentHashMismatch {
        /// 페이지 id.
        id: String,
        /// 호출자가 전달한 기대 hash.
        expected: String,
        /// 디스크의 실제 hash.
        actual: String,
    },

    /// 알 수 없는 페이지 타입.
    #[error("unknown page type: {0}")]
    UnknownPageType(String),

    /// 유효하지 않은 페이지 id.
    #[error("invalid page id: {0}")]
    InvalidPageId(String),

    /// JSON 직렬화 실패.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 메모리 crate (임베딩) 에러.
    #[error("embed error: {0}")]
    Embed(String),

    /// 기타.
    #[error("{0}")]
    Other(String),
}

/// 위키 모듈 결과 타입.
pub type Result<T> = std::result::Result<T, WikiError>;

/// 콘텐츠 SHA-256 해시 계산 (hex lowercase).
pub fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic() {
        let a = content_hash("hello world");
        let b = content_hash("hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn content_hash_differs() {
        assert_ne!(content_hash("a"), content_hash("b"));
    }
}
