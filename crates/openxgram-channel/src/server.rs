//! ChannelServer — axum 기반 Starian channel-mcp 호환 HTTP 서버.
//!
//! 엔드포인트 (모두 POST JSON, 단순 REST 형태로 MCP 도구 호출 매핑):
//! - GET  /v1/health
//! - POST /tools/send_to_platform        { platform, channel_id, text, reply_to? }
//! - POST /tools/send_message            { to, summary, type, details? }
//! - POST /tools/list_adapters           {}
//! - POST /tools/list_peers              {}
//! - POST /tools/set_status              { status, summary? }
//! - POST /tools/track_task              { task_id, title, status? }
//!
//! 외부 channel-mcp 클라이언트가 동일 도구 시그니처로 호출 가능.
//!
//! 인증: 옵션 Bearer 토큰. 없으면 인증 없음 (기본 127.0.0.1 만 허용).
//!
//! 절대 규칙: 0.0.0.0 바인딩 금지. 호출자가 SocketAddr 으로 전달, 본 모듈은 검증만.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::route::RouteEngine;
use crate::{ChannelError, Result};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub auth_token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    route: RouteEngine,
    auth_token: Option<String>,
    status: Arc<Mutex<StatusEntry>>,
    tasks: Arc<Mutex<Vec<TaskEntry>>>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct StatusEntry {
    status: String,
    summary: Option<String>,
    at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TaskEntry {
    task_id: String,
    title: String,
    status: String,
    at: String,
}

#[derive(Debug)]
pub struct ServerHandle {
    pub bound_addr: SocketAddr,
    pub join: JoinHandle<()>,
}

pub struct ChannelServer;

impl ChannelServer {
    pub async fn serve(config: ServerConfig, route: RouteEngine) -> Result<ServerHandle> {
        serve(config, route).await
    }
}

pub async fn serve(config: ServerConfig, route: RouteEngine) -> Result<ServerHandle> {
    enforce_loopback(&config.bind)?;

    let state = AppState {
        route,
        auth_token: config.auth_token,
        status: Arc::new(Mutex::new(StatusEntry::default())),
        tasks: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/v1/health", get(health))
        .route("/tools/send_to_platform", post(t_send_to_platform))
        .route("/tools/send_message", post(t_send_message))
        .route("/tools/list_adapters", post(t_list_adapters))
        .route("/tools/list_peers", post(t_list_peers))
        .route("/tools/set_status", post(t_set_status))
        .route("/tools/track_task", post(t_track_task))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let bound_addr = listener.local_addr()?;
    let join = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "channel server stopped");
        }
    });

    Ok(ServerHandle { bound_addr, join })
}

/// 절대 규칙: 0.0.0.0 / :: 등 외부 바인딩 금지. 127.0.0.1, ::1, 또는 명시적 link-local 만 허용.
fn enforce_loopback(addr: &SocketAddr) -> Result<()> {
    let ip = addr.ip();
    let ok = match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !ok {
        return Err(ChannelError::Invalid(format!(
            "non-loopback bind forbidden: {addr} (must be 127.0.0.1 or ::1)"
        )));
    }
    Ok(())
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> std::result::Result<(), StatusCode> {
    let Some(want) = state.auth_token.as_ref() else {
        return Ok(());
    };
    let got = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match got {
        Some(t) if t == want => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (
        code,
        Json(ErrorBody {
            ok: false,
            error: msg.into(),
        }),
    )
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "service": "openxgram-channel" }))
}

// ── tools ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SendToPlatformReq {
    platform: String,
    channel_id: String,
    text: String,
    #[serde(default)]
    reply_to: Option<String>,
}

async fn t_send_to_platform(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SendToPlatformReq>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    match state
        .route
        .send_to_platform(
            &req.platform,
            &req.channel_id,
            &req.text,
            req.reply_to.as_deref(),
        )
        .await
    {
        Ok(r) => Json(serde_json::json!({ "ok": true, "result": r })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SendMessageReq {
    to: String,
    summary: String,
    #[serde(default = "default_msg_type", rename = "type")]
    msg_type: String,
    #[serde(default)]
    details: Option<String>,
}

fn default_msg_type() -> String {
    "info".into()
}

async fn t_send_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SendMessageReq>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    match state
        .route
        .send_message(&req.to, &req.summary, &req.msg_type, req.details.as_deref())
        .await
    {
        Ok(r) => Json(serde_json::json!({ "ok": true, "result": r })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn t_list_adapters(
    State(state): State<AppState>,
    headers: HeaderMap,
    _body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    let list = state.route.adapters.list().await;
    Json(serde_json::json!({ "ok": true, "adapters": list })).into_response()
}

async fn t_list_peers(
    State(state): State<AppState>,
    headers: HeaderMap,
    _body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    let peers = state.route.peers.list().await;
    Json(serde_json::json!({ "ok": true, "peers": peers })).into_response()
}

#[derive(Debug, Deserialize)]
struct SetStatusReq {
    status: String,
    #[serde(default)]
    summary: Option<String>,
}

async fn t_set_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SetStatusReq>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    let mut g = state.status.lock().await;
    g.status = req.status;
    g.summary = req.summary;
    g.at = Some(openxgram_core::time::kst_now().to_rfc3339());
    Json(serde_json::json!({ "ok": true, "status": *g })).into_response()
}

#[derive(Debug, Deserialize)]
struct TrackTaskReq {
    task_id: String,
    title: String,
    #[serde(default = "default_task_status")]
    status: String,
}

fn default_task_status() -> String {
    "pending".into()
}

async fn t_track_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TrackTaskReq>,
) -> impl IntoResponse {
    if let Err(c) = check_auth(&state, &headers) {
        return err(c, "unauthorized").into_response();
    }
    let entry = TaskEntry {
        task_id: req.task_id,
        title: req.title,
        status: req.status,
        at: openxgram_core::time::kst_now().to_rfc3339(),
    };
    state.tasks.lock().await.push(entry.clone());
    Json(serde_json::json!({ "ok": true, "task": entry })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_accepted() {
        let a: SocketAddr = "127.0.0.1:7250".parse().unwrap();
        assert!(enforce_loopback(&a).is_ok());
        let b: SocketAddr = "[::1]:7250".parse().unwrap();
        assert!(enforce_loopback(&b).is_ok());
    }

    #[test]
    fn external_bind_rejected() {
        let a: SocketAddr = "0.0.0.0:7250".parse().unwrap();
        assert!(enforce_loopback(&a).is_err());
        let b: SocketAddr = "192.168.1.5:7250".parse().unwrap();
        assert!(enforce_loopback(&b).is_err());
    }
}
