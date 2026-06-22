//! 사이드카가 점유하는 포트 — manifest·init·doctor 가 공유.

// 7300/7301 은 IANA 등록(XPilot 등) 영역 — 충돌 가능성. 47300/47301 로 이동.
// 47xxx 대는 IANA 미등록·비공개 영역이라 일반 데스크톱·서버에서 거의 빈 상태.
pub const RPC_PORT: u16 = 47300;
pub const HTTP_PORT: u16 = 47301;
/// daemon-gui HTTP API (`/v1/gui/*`) — Tauri 데스크톱 앱·기타 클라이언트용.
pub const GUI_PORT: u16 = 47302;
/// 잘만(zalman) 등 17400 transport 계열 머신의 GUI 포트(17400→17402, +2 규칙).
/// 설치마다 GUI 포트가 다르므로 cross-machine 후보 탐색에 사용.
pub const ZALMAN_GUI_PORT: u16 = 17402;
pub const INBOUND_WEBHOOK_PORT: u16 = 14921;

/// cross-machine GUI 후보 포트 — 설치별 GUI 포트가 다를 때 순차 탐색.
pub const GUI_CANDIDATE_PORTS: &[u16] = &[GUI_PORT, ZALMAN_GUI_PORT];

pub const RPC_SERVICE: &str = "xgram-rpc";
pub const HTTP_SERVICE: &str = "xgram-http";
/// systemd --user 사이드카 데몬 서비스 유닛 이름(설치·업데이트·재시작 공유).
pub const SIDECAR_SERVICE: &str = "openxgram-sidecar.service";

/// 기본 loopback transport bind 문자열 — clap `default_value`(=&'static str)용 단일 정의.
/// `RPC_PORT` 와 동기화되어야 한다(아래 `bind_defaults_match_ports` 테스트가 강제).
pub const RPC_BIND_DEFAULT: &str = "127.0.0.1:47300";
/// 기본 loopback transport URL — clap `default_value`용.
pub const RPC_URL_DEFAULT: &str = "http://127.0.0.1:47300";
/// 기본 loopback GUI bind 문자열 — clap `default_value`용.
pub const GUI_BIND_DEFAULT: &str = "127.0.0.1:47302";

/// init 이 사전 점검하는 포트 목록.
pub const REQUIRED_PORTS: &[u16] = &[RPC_PORT, HTTP_PORT];

#[cfg(test)]
mod tests {
    use super::*;

    // SSOT 강제 — bind 문자열 상수가 숫자 포트 상수와 어긋나면(drift) 빌드 실패.
    #[test]
    fn bind_defaults_match_ports() {
        assert!(RPC_BIND_DEFAULT.ends_with(&format!(":{RPC_PORT}")));
        assert!(RPC_URL_DEFAULT.ends_with(&format!(":{RPC_PORT}")));
        assert!(GUI_BIND_DEFAULT.ends_with(&format!(":{GUI_PORT}")));
    }
}
