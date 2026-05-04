//! openxgram-orchestration — 예약 메시지 + 체인 메시지
//!
//! 다중 에이전트 워크플로우의 시간 기반 / 조건 기반 라우팅.
//!
//! - [`scheduled`] — `ScheduledMessage` / `ScheduledStore` (once + cron, KST)
//! - [`chain`] — `MessageChain` / `ChainStep` / `ChainStore` / `ChainRunner`
//!
//! 모든 타임스탬프는 KST (Asia/Seoul) 기준 epoch seconds.
//! `silent fallback` 금지 — 실패 시 `last_error` 컬럼에 명시 + `status='failed'`.

pub mod chain;
pub mod scheduled;

pub use chain::{
    ChainDefinition, ChainRunResult, ChainRunner, ChainStep, ChainStepInput, ChainStepResult,
    ChainStore, ChannelSender, ConditionKind, MessageChain, NoopSender,
};
pub use scheduled::{
    compute_next_due_kst, kst_now_epoch, parse_iso_kst, ScheduleKind, ScheduledMessage,
    ScheduledStatus, ScheduledStore, TargetKind,
};

#[derive(Debug, thiserror::Error)]
pub enum OrchestrationError {
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("invalid cron expression: {0}")]
    InvalidCron(String),

    #[error("invalid datetime: {0}")]
    InvalidDateTime(String),

    #[error("chain `{0}` not found")]
    ChainNotFound(String),

    #[error("scheduled message `{0}` not found")]
    ScheduledNotFound(String),

    #[error("yaml parse: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("json parse: {0}")]
    Json(#[from] serde_json::Error),

    #[error("send failed: {0}")]
    Send(String),

    #[error("invalid target kind: {0}")]
    InvalidTargetKind(String),

    #[error("invalid status: {0}")]
    InvalidStatus(String),

    #[error("invalid schedule kind: {0}")]
    InvalidScheduleKind(String),
}

pub type Result<T> = std::result::Result<T, OrchestrationError>;
