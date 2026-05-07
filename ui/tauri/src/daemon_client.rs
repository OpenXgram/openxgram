//! Daemon HTTP 클라이언트 — Tauri 핸들러가 원격 daemon 의 `/v1/gui/*` API 호출.
//!
//! 모드:
//!   - **로컬 (default)**: env `XGRAM_DAEMON_URL` 미설정 — 핸들러는 lib 직접 호출 (기존 동작).
//!   - **원격**: env `XGRAM_DAEMON_URL` 설정 시 (예: `http://100.x.x.x:47302`) HTTP 호출.
//!
//! 인증: env `XGRAM_DAEMON_TOKEN` (Bearer). 미설정·서버에서 require_auth 끈 dev 환경 시 생략 가능.
//!
//! 절대 규칙: silent fallback 금지 — env 있는데 호출 실패 시 raise (lib fallback 안 함).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct StatusDto {
    pub initialized: bool,
    pub alias: Option<String>,
    pub address: Option<String>,
    pub data_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct PeerDto {
    pub id: String,
    pub alias: String,
    pub address: String,
    pub public_key_hex: String,
    pub role: String,
    pub created_at: String,
    pub last_seen: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelAdapterStatus {
    pub platform: String,
    pub configured: bool,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelStatusDto {
    pub adapters: Vec<ChannelAdapterStatus>,
    pub peer_count: usize,
    pub schedule_pending: usize,
}

impl DaemonClient {
    /// env 기반 — `XGRAM_DAEMON_URL` 없으면 None (로컬 모드).
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("XGRAM_DAEMON_URL").ok()?;
        if url.trim().is_empty() {
            return None;
        }
        let token = std::env::var("XGRAM_DAEMON_TOKEN").ok().filter(|t| !t.is_empty());
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .ok()?;
        Some(Self {
            base_url: url.trim_end_matches('/').to_string(),
            token,
            http,
        })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self.http.request(method, format!("{}{path}", self.base_url));
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        r
    }

    pub async fn health(&self) -> Result<bool, String> {
        let r = self.req(reqwest::Method::GET, "/v1/gui/health")
            .send()
            .await
            .map_err(|e| format!("daemon health 호출 실패: {e}"))?;
        Ok(r.status().is_success())
    }

    pub async fn status(&self) -> Result<StatusDto, String> {
        self.req(reqwest::Method::GET, "/v1/gui/status")
            .send()
            .await
            .map_err(|e| format!("daemon /v1/gui/status: {e}"))?
            .error_for_status()
            .map_err(|e| format!("status HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("status JSON: {e}"))
    }

    pub async fn initialized(&self) -> Result<bool, String> {
        self.req(reqwest::Method::GET, "/v1/gui/initialized")
            .send()
            .await
            .map_err(|e| format!("daemon /v1/gui/initialized: {e}"))?
            .error_for_status()
            .map_err(|e| format!("initialized HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("initialized JSON: {e}"))
    }

    pub async fn peers(&self) -> Result<Vec<PeerDto>, String> {
        self.req(reqwest::Method::GET, "/v1/gui/peers")
            .send()
            .await
            .map_err(|e| format!("daemon /v1/gui/peers: {e}"))?
            .error_for_status()
            .map_err(|e| format!("peers HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("peers JSON: {e}"))
    }

    pub async fn channel_status(&self) -> Result<ChannelStatusDto, String> {
        self.req(reqwest::Method::GET, "/v1/gui/channel/status")
            .send()
            .await
            .map_err(|e| format!("daemon /v1/gui/channel/status: {e}"))?
            .error_for_status()
            .map_err(|e| format!("channel/status HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("channel/status JSON: {e}"))
    }

    pub async fn peer_add(&self, body: &PeerAddBody) -> Result<PeerDto, String> {
        self.req(reqwest::Method::POST, "/v1/gui/peers")
            .json(body)
            .send()
            .await
            .map_err(|e| format!("daemon POST /v1/gui/peers: {e}"))?
            .error_for_status()
            .map_err(|e| format!("peer_add HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("peer_add JSON: {e}"))
    }

    pub async fn vault_pending_list(&self) -> Result<Vec<PendingDto>, String> {
        self.req(reqwest::Method::GET, "/v1/gui/vault/pending")
            .send()
            .await
            .map_err(|e| format!("daemon vault/pending: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JSON: {e}"))
    }

    pub async fn vault_pending_approve(&self, id: &str) -> Result<(), String> {
        self.req(
            reqwest::Method::POST,
            &format!("/v1/gui/vault/pending/{id}/approve"),
        )
        .send()
        .await
        .map_err(|e| format!("daemon approve: {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP: {e}"))?;
        Ok(())
    }

    pub async fn vault_pending_deny(&self, id: &str, reason: Option<String>) -> Result<(), String> {
        let body = serde_json::json!({ "reason": reason });
        self.req(
            reqwest::Method::POST,
            &format!("/v1/gui/vault/pending/{id}/deny"),
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("daemon deny: {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP: {e}"))?;
        Ok(())
    }

    pub async fn payment_get_daily_limit(&self) -> Result<i64, String> {
        self.req(reqwest::Method::GET, "/v1/gui/payment/daily-limit")
            .send()
            .await
            .map_err(|e| format!("daemon daily-limit: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JSON: {e}"))
    }

    pub async fn payment_set_daily_limit(&self, micro_usdc: i64) -> Result<(), String> {
        self.req(reqwest::Method::PUT, "/v1/gui/payment/daily-limit")
            .json(&DailyLimitBody { micro_usdc })
            .send()
            .await
            .map_err(|e| format!("daemon set daily-limit: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP: {e}"))?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct PeerAddBody {
    pub alias: String,
    pub address: String,
    pub public_key_hex: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PendingDto {
    pub id: String,
    pub key: String,
    pub agent: String,
    pub action: String,
    pub status: String,
    pub requested_at: String,
}

#[derive(Debug, Serialize)]
struct DailyLimitBody {
    pub micro_usdc: i64,
}

#[derive(Debug, Deserialize)]
pub struct NotifyStatusDto {
    pub telegram_configured: bool,
    pub discord_configured: bool,
    pub discord_webhook_configured: bool,
}

impl DaemonClient {
    pub async fn notify_status(&self) -> Result<NotifyStatusDto, String> {
        self.req(reqwest::Method::GET, "/v1/gui/notify/status")
            .send()
            .await
            .map_err(|e| format!("daemon notify/status: {e}"))?
            .error_for_status()
            .map_err(|e| format!("HTTP: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JSON: {e}"))
    }
}
