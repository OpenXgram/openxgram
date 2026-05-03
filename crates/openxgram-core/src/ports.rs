//! 사이드카가 점유하는 포트 — manifest·init·doctor 가 공유.

pub const RPC_PORT: u16 = 7300;
pub const HTTP_PORT: u16 = 7301;
pub const INBOUND_WEBHOOK_PORT: u16 = 14921;

pub const RPC_SERVICE: &str = "xgram-rpc";
pub const HTTP_SERVICE: &str = "xgram-http";

/// init 이 사전 점검하는 포트 목록.
pub const REQUIRED_PORTS: &[u16] = &[RPC_PORT, HTTP_PORT];
