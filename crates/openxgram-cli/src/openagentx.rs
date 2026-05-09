//! step 11/12 — OpenAgentX 마켓플레이스 bridge.
//!
//! 무료 (step 11):  `xgram openagentx call <agent> <prompt>` — 응답 받기
//! 유료 (step 12):  `xgram openagentx call <agent> <prompt> --pay 50000` — payment_intent draft 생성 후 호출
//!
//! 환경변수:
//!   XGRAM_OPENAGENTX_URL    — 마켓플레이스 base URL (예: https://api.openagentx.org)
//!   XGRAM_OPENAGENTX_TOKEN  — 인증 토큰 (Bearer)
//!
//! 응답은 inbox-from-openagentx:<agent> 세션에 메모리 저장.

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CallOpts {
    pub agent: String,
    pub prompt: String,
    /// 유료 호출이면 micro USDC 금액 (1 USDC = 1_000_000 micro). None 이면 무료.
    pub pay_micros: Option<u64>,
    /// 결제 메모 (payment_intents.memo)
    pub memo: Option<String>,
}

#[derive(Debug, Serialize)]
struct InvokeReq<'a> {
    agent: &'a str,
    prompt: &'a str,
    /// 결제 의도 ID (유료 호출 시) — 마켓플레이스가 onchain 확인 후 처리
    #[serde(skip_serializing_if = "Option::is_none")]
    payment_intent_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct InvokeResp {
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// 마켓플레이스 측 receipt id (옵션)
    #[serde(default)]
    receipt_id: Option<String>,
}

pub async fn run_call(data_dir: &Path, opts: CallOpts) -> Result<String> {
    let base_url = std::env::var("XGRAM_OPENAGENTX_URL")
        .context("XGRAM_OPENAGENTX_URL env 필요 (마켓플레이스 URL)")?;
    let token = std::env::var("XGRAM_OPENAGENTX_TOKEN").ok();
    if opts.agent.trim().is_empty() {
        bail!("agent 비어있음 (예: @translator-pro)");
    }
    if opts.prompt.trim().is_empty() {
        bail!("prompt 비어있음");
    }

    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;

    // 1) 유료 호출이면 PaymentStore.create_draft (서명/제출은 사용자가 별도 — 본 PR 은 draft 까지)
    let payment_intent_id = if let Some(amount) = opts.pay_micros {
        if amount == 0 {
            None
        } else {
            let intent_id = create_payment_draft(&mut db, &opts.agent, amount, opts.memo.as_deref())
                .context("payment_intent draft")?;
            eprintln!("[openagentx] payment_intent draft: {intent_id} ({amount} micro USDC). 서명/제출은 별도 (`xgram payment ...`).");
            Some(intent_id)
        }
    } else {
        None
    };

    // 2) HTTP 호출
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let url = format!("{}/agents/invoke", base_url.trim_end_matches('/'));
    let req = InvokeReq {
        agent: &opts.agent,
        prompt: &opts.prompt,
        payment_intent_id: payment_intent_id.as_deref(),
    };
    let mut req_builder = http.post(&url).json(&req);
    if let Some(t) = token.as_deref() {
        req_builder = req_builder.bearer_auth(t);
    }
    let resp = req_builder.send().await.context("openagentx POST")?;
    if !resp.status().is_success() {
        let st = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("openagentx HTTP {st}: {body}");
    }
    let parsed: InvokeResp = resp.json().await.context("openagentx JSON parse")?;
    let answer = parsed
        .response
        .or(parsed.text)
        .unwrap_or_else(|| "(no text response)".into());

    // 3) 응답 메모리 저장
    let session_title = format!("inbox-from-openagentx:{}", opts.agent);
    let session = SessionStore::new(&mut db)
        .ensure_by_title(&session_title, "openagentx")
        .context("openagentx session ensure")?;
    let embedder = default_embedder()?;
    MessageStore::new(&mut db, embedder.as_ref())
        .insert(
            &session.id,
            &format!("openagentx:{}", opts.agent),
            &answer,
            "openagentx",
            None,
        )
        .context("openagentx response insert")?;

    eprintln!("[openagentx] ← {} ({} chars). receipt={:?}", opts.agent, answer.len(), parsed.receipt_id);
    Ok(answer)
}

fn create_payment_draft(
    db: &mut Db,
    agent: &str,
    amount_micros: u64,
    memo: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let now = openxgram_core::time::kst_now().to_rfc3339();
    // payee_address 는 알 수 없음 (마켓플레이스가 응답에 포함해야) — placeholder.
    let payee = format!("openagentx:{agent}");
    let memo_txt = memo.unwrap_or("openagentx call");
    db.conn().execute(
        "INSERT INTO payment_intents
            (id, amount_usdc_micro, chain, payee_address, memo, nonce, state, created_at)
         VALUES (?1, ?2, 'base', ?3, ?4, ?5, 'draft', ?6)",
        rusqlite::params![id, amount_micros as i64, payee, memo_txt, nonce, now],
    )?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_core::paths::manifest_path;
    use tempfile::tempdir;

    fn open_test_dir() -> tempfile::TempDir {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();
        tmp
    }

    #[tokio::test]
    async fn call_rejects_empty_args() {
        let tmp = open_test_dir();
        let dir = tmp.path();
        unsafe {
            std::env::set_var("XGRAM_OPENAGENTX_URL", "http://example");
        }
        let res = run_call(
            dir,
            CallOpts {
                agent: "".into(),
                prompt: "x".into(),
                pay_micros: None,
                memo: None,
            },
        )
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn call_rejects_missing_url() {
        let tmp = open_test_dir();
        let dir = tmp.path();
        unsafe {
            std::env::remove_var("XGRAM_OPENAGENTX_URL");
        }
        let res = run_call(
            dir,
            CallOpts {
                agent: "@x".into(),
                prompt: "p".into(),
                pay_micros: None,
                memo: None,
            },
        )
        .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("XGRAM_OPENAGENTX_URL"));
    }

    #[tokio::test]
    async fn call_with_payment_creates_draft_in_db() {
        // mock OpenAgentX 서버
        use axum::routing::post;
        use axum::{Json, Router};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        async fn handler(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "response": format!("got prompt={}", body["prompt"]),
                "receipt_id": "r-1"
            }))
        }
        let app = Router::new().route("/agents/invoke", post(handler));
        let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tmp = open_test_dir();
        let dir = tmp.path();
        // init schema
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        drop(db);

        unsafe {
            std::env::set_var("XGRAM_OPENAGENTX_URL", &format!("http://127.0.0.1:{port}"));
            std::env::remove_var("XGRAM_OPENAGENTX_TOKEN");
        }
        let answer = run_call(
            dir,
            CallOpts {
                agent: "@translator-pro".into(),
                prompt: "안녕".into(),
                pay_micros: Some(50_000), // 0.05 USDC
                memo: Some("translation".into()),
            },
        )
        .await
        .unwrap();
        assert!(answer.contains("got prompt"));

        // payment_intents 에 draft row 1
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM payment_intents WHERE state='draft' AND amount_usdc_micro=?1",
                [50_000i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
