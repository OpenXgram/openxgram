//! openxgram-core — 공통 상수·경로·시간·환경변수 hub.
//!
//! 모든 다른 crate 가 여기서만 import 한다. 같은 상수/경로 계산이 여러 곳에
//! 흩어지지 않도록 단일 source of truth.

pub mod confirm;
pub mod env;
pub mod paths;
pub mod ports;
pub mod time;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("HOME 환경변수 누락 (Windows: USERPROFILE 도 미설정)")]
    NoHome,

    #[error("환경변수 {0} 누락")]
    MissingEnv(&'static str),
}

pub type Result<T> = std::result::Result<T, CoreError>;
