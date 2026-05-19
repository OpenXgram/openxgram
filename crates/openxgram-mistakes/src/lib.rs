//! openxgram-mistakes — 실수 레지스트리 (W의 규칙 1, 회고형 기억).
//!
//! 정본: docs/PRD-OpenXgram.md §4.2
//!
//! "내가 한 모든 것을 체계적으로 기록하고 벡터 검색해서, 같은 실수를 반복하지 않는 것."
//!
//! 4 MCP 도구:
//!   - `check_for_mistakes(planned_action, k?)`  → 유사 과거 실수 top-K + 경고
//!   - `log_mistake({intended, outcome, reason, lesson, severity?, related_wiki?})` → 등록
//!   - `find_similar_failures(situation, k?)`   → 유사 실수 검색
//!   - `resolve_mistake(mistake_id, resolution)` → 해결됨 표시
//!
//! 모듈:
//!   - mistake : 도메인 객체 (Mistake, MistakeId, NewMistake)
//!   - store   : DB CRUD + LIKE 검색
//!   - mcp     : 4 도구 핸들러

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod mcp;
pub mod mistake;
pub mod store;

pub use mcp::MistakeTools;
pub use mistake::{Mistake, MistakeId, NewMistake};
pub use store::{MistakeStore, MistakeStoreError};

use thiserror::Error;

/// 실수 레지스트리 전반 에러.
#[derive(Debug, Error)]
pub enum MistakesError {
    /// DB 에러.
    #[error("store: {0}")]
    Store(#[from] MistakeStoreError),

    /// 입력 검증 실패.
    #[error("invalid: {0}")]
    Invalid(String),

    /// 찾을 수 없음.
    #[error("not found: {0}")]
    NotFound(String),

    /// 일반.
    #[error("{0}")]
    Other(String),
}
