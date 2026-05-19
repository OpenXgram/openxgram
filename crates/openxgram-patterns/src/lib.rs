//! openxgram-patterns — 패턴 매칭 제안 엔진 (W의 규칙 2, 예측형 기억).
//!
//! 정본: docs/PRD-OpenXgram.md §4.3
//!
//! "내 행동을 패턴화하여, 새로운 행동이 어떤 패턴과 유사한지 매칭해서 다음 행동을 제안하는 것."
//!
//! 기존 자산: L3 patterns (0004) NEW/RECURRING/ROUTINE + frequency.
//! 본 crate는 action_sequence + 성공률 추적 + 임베딩 인덱스 (0020).
//!
//! 4 MCP 도구:
//!   - `match_action_pattern(new_action, k?, min_similarity?)` → 유사 패턴 top-K
//!   - `suggest_next_steps(current_state)`                     → 매칭 패턴의 다음 단계 제안
//!   - `confirm_pattern_execution(pattern_id, modifications?)` → 실행 시작 (modifications 옵션)
//!   - `record_pattern_outcome(pattern_id, success, duration_ms?)` → 결과 기록
//!
//! 모듈:
//!   - pattern : 도메인 객체 (ActionPattern, ActionPatternId, ActionStep, NewActionPattern)
//!   - store   : DB CRUD + LIKE 검색
//!   - mcp     : 4 도구 핸들러

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod mcp;
pub mod pattern;
pub mod store;

pub use mcp::PatternTools;
pub use pattern::{ActionPattern, ActionPatternId, ActionStep, NewActionPattern};
pub use store::{ActionPatternStore, ActionPatternStoreError};

use thiserror::Error;

/// 모듈 전체 에러.
#[derive(Debug, Error)]
pub enum PatternsError {
    /// 저장소.
    #[error("store: {0}")]
    Store(#[from] ActionPatternStoreError),

    /// 입력 검증.
    #[error("invalid: {0}")]
    Invalid(String),

    /// 못 찾음.
    #[error("not found: {0}")]
    NotFound(String),

    /// JSON.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// 기타.
    #[error("{0}")]
    Other(String),
}
