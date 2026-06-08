//! rc.277 — Paperclip orchestration absorption, Phase 2 (adapter abstraction).
//!
//! Mirrors paperclip's `AdapterExecutionContext` / `AdapterExecutionResult`
//! (`packages/adapter-utils/src/types.ts`) + adapter registry
//! (`server/src/adapters/registry.ts`) as a Rust trait + dispatch.
//!
//! 3 adapters:
//!   - `peer_send`  : reuse `crate::peer_send::run_peer_send_with_conv` to send the prompt to a
//!                    fleet peer, then poll the `inbox-from-{alias}` session for the reply.
//!                    **Messaging is NOT reimplemented** — send + reply collection reuse the
//!                    existing peer_send / inbound (messages table) plumbing.
//!   - `process`    : spawn a local command, collect stdout.
//!   - `http`       : POST the prompt body to a webhook URL, return the response body.
//!
//! 절대 규칙 (CLAUDE.md): no silent fallback (errors are surfaced), no production `unwrap()`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};

/// Partial-output stream callback (mirrors paperclip `onLog(stream, chunk)`).
pub type OnLog = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Adapter invocation context — mirror of paperclip `AdapterExecutionContext`.
pub struct AdapterContext {
    /// OpenXgram data_dir (DB / keystore / manifest root).
    pub data_dir: PathBuf,
    /// Target agent alias (org-chart node alias).
    pub agent_alias: String,
    /// Prompt / issue body to deliver to the agent.
    pub prompt: String,
    /// adapter_config JSON (agent_capabilities.adapter_config), e.g. {"alias":"..."}.
    pub adapter_config: serde_json::Value,
    /// Keystore password (peer_send signing). Required for `peer_send` adapter.
    pub password: Option<String>,
    /// Session id for continuity (mirror of paperclip runtime.sessionId).
    pub session_id: Option<String>,
    /// Hard timeout for the whole execution.
    pub timeout: Duration,
    /// Partial-output stream callback (optional).
    pub on_log: Option<OnLog>,
}

impl AdapterContext {
    fn log(&self, stream: &str, chunk: &str) {
        if let Some(cb) = &self.on_log {
            cb(stream, chunk);
        }
    }
}

/// Adapter execution result — mirror of paperclip `AdapterExecutionResult`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AdapterResult {
    /// Reply text (paperclip `summary`).
    pub summary: String,
    /// Token usage if known (input, output) — optional.
    pub usage: Option<AdapterUsage>,
    /// Cost in USD if known — optional.
    pub cost_usd: Option<f64>,
    /// Session id to persist for continuity — optional.
    pub session_id: Option<String>,
    /// Adapter is asking a human (paperclip `question`) — optional.
    pub question: Option<String>,
    /// Whether the run timed out before a reply was collected.
    pub timed_out: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct AdapterUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// Adapter trait — `async fn execute(ctx) -> Result<AdapterResult>`.
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    async fn execute(&self, ctx: &AdapterContext) -> Result<AdapterResult>;
}

// ---------------------------------------------------------------------------
// peer_send adapter — reuses existing peer_send + inbound (messages) plumbing.
// ---------------------------------------------------------------------------

pub struct PeerSendAdapter;

#[async_trait::async_trait]
impl Adapter for PeerSendAdapter {
    async fn execute(&self, ctx: &AdapterContext) -> Result<AdapterResult> {
        // 1) Resolve target alias: adapter_config.alias overrides agent_alias.
        let target = ctx
            .adapter_config
            .get("alias")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| ctx.agent_alias.clone());

        let password = ctx
            .password
            .as_deref()
            .ok_or_else(|| anyhow!("peer_send adapter: keystore password 미설정 (XGRAM_KEYSTORE_PASSWORD)"))?;

        // 2) Mark the send boundary so we only collect replies that arrive AFTER it.
        let since = chrono::Utc::now().to_rfc3339();
        let conversation_id = ctx.session_id.clone();

        ctx.log("system", &format!("peer_send → {target}"));

        // 3) Reuse existing send path (signs + outbox + ACK tracking). No reimplementation.
        crate::peer_send::run_peer_send_with_conv(
            &ctx.data_dir,
            &target,
            None,
            &ctx.prompt,
            password,
            conversation_id.clone(),
        )
        .await
        .with_context(|| format!("peer_send adapter: run_peer_send_with_conv 실패 (alias={target})"))?;

        // 4) Poll the inbox-from-{alias} session for the reply (reuse messages table).
        let start = Instant::now();
        let poll_every = Duration::from_secs(2);
        loop {
            if start.elapsed() >= ctx.timeout {
                ctx.log("system", "peer_send: reply timeout");
                return Ok(AdapterResult {
                    summary: String::new(),
                    session_id: conversation_id,
                    timed_out: true,
                    ..Default::default()
                });
            }
            tokio::time::sleep(poll_every).await;
            if let Some(reply) = poll_inbox_reply(&ctx.data_dir, &target, &since, conversation_id.as_deref())? {
                ctx.log("stdout", &reply);
                return Ok(AdapterResult {
                    summary: reply,
                    session_id: conversation_id,
                    ..Default::default()
                });
            }
        }
    }
}

/// Look for the newest inbound message from `target` after `since` (rfc3339).
/// Inbound replies land in the `inbox-from-{alias}` session (daemon.process_inbound),
/// sender label `peer:{alias}` / `unverified:{alias}`. If a conversation_id is given,
/// prefer a message that carries it; otherwise fall back to newest-after-since.
fn poll_inbox_reply(
    data_dir: &Path,
    alias: &str,
    since: &str,
    conversation_id: Option<&str>,
) -> Result<Option<String>> {
    let mut db = open_db(data_dir)?;
    let conn = db.conn();
    let inbox_title = format!("inbox-from-{}", alias);

    // conversation_id 우선 매칭 (정확) → 없으면 since 이후 최신 inbound.
    if let Some(conv) = conversation_id {
        let row: Option<String> = conn
            .query_row(
                "SELECT m.body FROM sessions s JOIN messages m ON m.session_id = s.id \
                 WHERE s.title = ?1 AND m.conversation_id = ?2 AND m.timestamp > ?3 \
                 ORDER BY m.timestamp DESC LIMIT 1",
                rusqlite::params![inbox_title, conv, since],
                |r| r.get::<_, String>(0),
            )
            .ok();
        if row.is_some() {
            return Ok(row);
        }
    }

    let row: Option<String> = conn
        .query_row(
            "SELECT m.body FROM sessions s JOIN messages m ON m.session_id = s.id \
             WHERE s.title = ?1 AND m.timestamp > ?2 \
             ORDER BY m.timestamp DESC LIMIT 1",
            rusqlite::params![inbox_title, since],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(row)
}

// ---------------------------------------------------------------------------
// process adapter — spawn a local command, collect stdout.
// ---------------------------------------------------------------------------

pub struct ProcessAdapter;

#[async_trait::async_trait]
impl Adapter for ProcessAdapter {
    async fn execute(&self, ctx: &AdapterContext) -> Result<AdapterResult> {
        let command = ctx
            .adapter_config
            .get("command")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("process adapter: adapter_config.command 필요"))?;

        let args: Vec<String> = ctx
            .adapter_config
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        ctx.log("system", &format!("process spawn: {command} {args:?}"));

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args);
        // Prompt is passed on stdin (paperclip process adapter convention).
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("process adapter: spawn 실패 ({command})"))?;

        // Write prompt to stdin.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(ctx.prompt.as_bytes())
                .await
                .context("process adapter: stdin write 실패")?;
            // drop stdin → EOF.
        }

        let output = tokio::time::timeout(ctx.timeout, child.wait_with_output()).await;
        let output = match output {
            Ok(o) => o.context("process adapter: wait_with_output 실패")?,
            Err(_) => {
                ctx.log("system", "process: timeout");
                return Ok(AdapterResult {
                    timed_out: true,
                    ..Default::default()
                });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            bail!(
                "process adapter: exit code {:?}, stderr={}",
                output.status.code(),
                stderr.trim()
            );
        }
        ctx.log("stdout", &stdout);
        Ok(AdapterResult {
            summary: stdout,
            ..Default::default()
        })
    }
}

// ---------------------------------------------------------------------------
// http adapter — POST prompt to a webhook URL, return response body.
// ---------------------------------------------------------------------------

pub struct HttpAdapter;

#[async_trait::async_trait]
impl Adapter for HttpAdapter {
    async fn execute(&self, ctx: &AdapterContext) -> Result<AdapterResult> {
        let url = ctx
            .adapter_config
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("http adapter: adapter_config.url 필요"))?;

        ctx.log("system", &format!("http POST → {url}"));

        let client = reqwest::Client::builder()
            .timeout(ctx.timeout)
            .build()
            .context("http adapter: client build 실패")?;

        let payload = serde_json::json!({
            "agent": ctx.agent_alias,
            "prompt": ctx.prompt,
            "session_id": ctx.session_id,
        });

        let resp = client
            .post(url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("http adapter: POST 실패 ({url})"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("http adapter: 응답 본문 읽기 실패")?;

        if !status.is_success() {
            bail!("http adapter: HTTP {status} — {}", body.chars().take(200).collect::<String>());
        }
        ctx.log("stdout", &body);
        Ok(AdapterResult {
            summary: body,
            ..Default::default()
        })
    }
}

// ---------------------------------------------------------------------------
// registry — adapter_type string → Adapter dispatch (paperclip registry.ts pattern).
// ---------------------------------------------------------------------------

/// Resolve an adapter implementation by its `adapter_type` string.
/// Unknown types are an explicit error (no silent fallback).
pub fn get_adapter(adapter_type: &str) -> Result<Box<dyn Adapter>> {
    match adapter_type {
        "peer_send" => Ok(Box::new(PeerSendAdapter)),
        "process" => Ok(Box::new(ProcessAdapter)),
        "http" => Ok(Box::new(HttpAdapter)),
        other => Err(anyhow!(
            "지원 안 되는 adapter_type: '{other}' (지원: peer_send | process | http)"
        )),
    }
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!("DB 미존재 ({}). `xgram init` 먼저 실행.", path.display());
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_adapter_known_types() {
        assert!(get_adapter("peer_send").is_ok());
        assert!(get_adapter("process").is_ok());
        assert!(get_adapter("http").is_ok());
    }

    #[test]
    fn get_adapter_unknown_errors() {
        let err = get_adapter("xmtp").unwrap_err();
        assert!(err.to_string().contains("지원 안 되는 adapter_type"));
    }

    #[test]
    fn peer_send_adapter_requires_password() {
        let ctx = AdapterContext {
            data_dir: std::path::PathBuf::from("/nonexistent"),
            agent_alias: "x".into(),
            prompt: "hi".into(),
            adapter_config: serde_json::json!({"alias": "x"}),
            password: None,
            session_id: None,
            timeout: Duration::from_secs(1),
            on_log: None,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(PeerSendAdapter.execute(&ctx));
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("password"));
    }
}
