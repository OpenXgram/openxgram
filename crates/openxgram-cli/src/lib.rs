//! xgram CLI library — init/uninstall/doctor 등의 핵심 흐름.
//! main.rs(바이너리)와 통합 테스트가 공유한다.

pub mod backup;
pub mod daemon;
pub mod doctor;
pub mod init;
pub mod mcp_serve;
pub mod memory;
pub mod migrate;
pub mod notify;
pub mod reset;
pub mod session;
pub mod status;
pub mod tui;
pub mod uninstall;
pub mod wizard;
