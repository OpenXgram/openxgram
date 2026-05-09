//! 4.3 — 첫 indexer 운영 (axum service skeleton).
//! `GET /search?q=...` — 등록된 identity 중 q substring 매칭 + DefaultRanker 점수순 반환.
//! 4.3.1 docker 이미지 + 호스트는 배포 단계 (k8s/Vercel/SSH) — 본 모듈은 router 만 제공.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::ranking::{DefaultRanker, IdentityScore, Rank};

#[derive(Debug, Clone, Default)]
pub struct IndexerState {
    pub identities: Arc<Mutex<HashMap<String, IdentityScore>>>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResp {
    pub query: String,
    pub count: usize,
    pub results: Vec<IdentityScore>,
}

pub fn router(state: IndexerState) -> Router {
    Router::new()
        .route("/search", get(search_handler))
        .route("/register", axum::routing::post(register_handler))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterReq {
    pub handle: String,
    /// optional self-reported counts (마켓플레이스가 별도 검증 가능)
    #[serde(default)]
    pub messages: u64,
    #[serde(default)]
    pub payments_received: u64,
    #[serde(default)]
    pub endorsements_received: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterResp {
    pub ok: bool,
    pub handle: String,
}

async fn register_handler(
    State(state): State<IndexerState>,
    axum::Json(body): axum::Json<RegisterReq>,
) -> Json<RegisterResp> {
    let mut map = state.identities.lock().await;
    map.insert(
        body.handle.clone(),
        IdentityScore {
            identity: body.handle.clone(),
            messages: body.messages,
            payments_received: body.payments_received,
            endorsements_received: body.endorsements_received,
            raw_score: 0.0,
        },
    );
    Json(RegisterResp {
        ok: true,
        handle: body.handle,
    })
}

async fn search_handler(
    State(state): State<IndexerState>,
    Query(q): Query<SearchQuery>,
) -> Json<SearchResp> {
    let map = state.identities.lock().await.clone();
    let needle = q.q.to_lowercase();
    let matches: Vec<IdentityScore> = map
        .into_values()
        .filter(|i| needle.is_empty() || i.identity.to_lowercase().contains(&needle))
        .collect();
    let ranked = DefaultRanker::default().rank(matches);
    Json(SearchResp {
        query: q.q,
        count: ranked.len(),
        results: ranked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pick_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    #[tokio::test]
    async fn search_returns_ranked_matches() {
        let state = IndexerState::default();
        {
            let mut m = state.identities.lock().await;
            m.insert(
                "alice.base.eth".into(),
                IdentityScore {
                    identity: "alice.base.eth".into(),
                    messages: 10,
                    payments_received: 0,
                    endorsements_received: 0,
                    raw_score: 0.0,
                },
            );
            m.insert(
                "bob.base.eth".into(),
                IdentityScore {
                    identity: "bob.base.eth".into(),
                    messages: 0,
                    payments_received: 0,
                    endorsements_received: 50,
                    raw_score: 0.0,
                },
            );
        }
        let port = pick_port();
        let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        let app = router(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp: SearchResp = reqwest::get(format!("http://127.0.0.1:{port}/search?q=base"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp.count, 2);
        // bob 이 endorsement 50 으로 랭크 1
        assert_eq!(resp.results[0].identity, "bob.base.eth");
    }

    #[tokio::test]
    async fn register_handler_inserts_identity() {
        let state = IndexerState::default();
        let port = pick_port();
        let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let body = serde_json::json!({
            "handle": "starian.base.eth",
            "messages": 100,
            "endorsements_received": 5
        });
        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/register"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let parsed: RegisterResp = resp.json().await.unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.handle, "starian.base.eth");

        // search 로 검색 가능
        let s: SearchResp = reqwest::get(format!("http://127.0.0.1:{port}/search?q=starian"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(s.count, 1);
        assert_eq!(s.results[0].messages, 100);
        assert_eq!(s.results[0].endorsements_received, 5);
    }

    #[tokio::test]
    async fn empty_query_returns_all() {
        let state = IndexerState::default();
        state.identities.lock().await.insert(
            "x".into(),
            IdentityScore {
                identity: "x".into(),
                ..Default::default()
            },
        );
        let port = pick_port();
        let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        let app = router(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp: SearchResp = reqwest::get(format!("http://127.0.0.1:{port}/search?q="))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp.count, 1);
    }
}
