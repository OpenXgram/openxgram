//! 사이드카가 점유하는 포트 — manifest·init·doctor 가 공유.

// 7300/7301 은 IANA 등록(XPilot 등) 영역 — 충돌 가능성. 47300/47301 로 이동.
// 47xxx 대는 IANA 미등록·비공개 영역이라 일반 데스크톱·서버에서 거의 빈 상태.
pub const RPC_PORT: u16 = 47300;
pub const HTTP_PORT: u16 = 47301;
pub const INBOUND_WEBHOOK_PORT: u16 = 14921;

pub const RPC_SERVICE: &str = "xgram-rpc";
pub const HTTP_SERVICE: &str = "xgram-http";

/// init 이 사전 점검하는 포트 목록.
pub const REQUIRED_PORTS: &[u16] = &[RPC_PORT, HTTP_PORT];
