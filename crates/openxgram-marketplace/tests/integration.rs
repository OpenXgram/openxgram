//! End-to-end 시나리오 — 마켓 API를 mockito로 stub.
//!
//! 검증:
//!   - search → get_agent → purchase (auto-approved) → get_job_status
//!   - 한도 초과 시 NeedsConfirmation (결제·job 생성 모두 X)
//!   - 화이트리스트 미등록 시 NeedsConfirmation

use std::sync::Arc;

use openxgram_marketplace::{
    AgentId, MarketplaceClient, MarketplaceTools, NewJobRequest, NoopPaymentGateway,
    PurchaseDecision, ServiceId, SpendPolicy,
};

fn make_tools(server_url: &str, policy: SpendPolicy) -> (MarketplaceTools, Arc<NoopPaymentGateway>) {
    let client = MarketplaceClient::builder()
        .base_url(server_url)
        .build()
        .unwrap();
    let gw = Arc::new(NoopPaymentGateway::new());
    let tools = MarketplaceTools::new(client, policy, gw.clone());
    (tools, gw)
}

#[tokio::test]
async fn search_then_get_agent_then_purchase_auto_approved() {
    let mut server = mockito::Server::new_async().await;

    // 1. search
    let m_search = server
        .mock("GET", "/api/agents")
        .match_query(mockito::Matcher::AnyOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "translate".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"agents":[
                {"id":"agent:tx1","name":"TranslateBot","description":"EN<->KO"}
            ]}"#,
        )
        .create_async()
        .await;

    // 2. get_agent — services 포함 (purchase 시 가격 lookup 용)
    let m_get = server
        .mock("GET", "/api/agents/agent:tx1")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "id":"agent:tx1",
                "name":"TranslateBot",
                "description":"EN<->KO",
                "services":[
                    {"id":"svc:en_ko","name":"EN→KO","description":"translate",
                     "price_usdc_micro":100000}
                ]
            }"#,
        )
        .expect_at_least(1)
        .create_async()
        .await;

    // 3. create_job
    let m_post = server
        .mock("POST", "/api/jobs")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "id":"job:001",
                "agent_id":"agent:tx1",
                "service_id":"svc:en_ko",
                "status":"queued",
                "created_at":"2026-05-18T00:00:00Z",
                "updated_at":"2026-05-18T00:00:00Z"
            }"#,
        )
        .create_async()
        .await;

    // 4. get_job
    let m_status = server
        .mock("GET", "/api/jobs/job:001")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "id":"job:001",
                "agent_id":"agent:tx1",
                "service_id":"svc:en_ko",
                "status":"completed",
                "output":{"text":"안녕"},
                "created_at":"2026-05-18T00:00:00Z",
                "updated_at":"2026-05-18T00:01:00Z"
            }"#,
        )
        .create_async()
        .await;

    let mut policy = SpendPolicy::default();
    // 화이트리스트에 추가 (자동 결제 허용)
    policy.allow(&AgentId("agent:tx1".into()));
    let (tools, gw) = make_tools(&server.url(), policy);

    // search
    let search = tools.search("translate", Some(5)).await.unwrap();
    assert_eq!(search.count, 1);
    assert_eq!(search.agents[0].id.as_str(), "agent:tx1");

    // get_agent
    let agent = tools.get_agent("agent:tx1").await.unwrap();
    assert_eq!(agent.services.len(), 1);
    assert_eq!(agent.services[0].price_usdc_micro, 100_000);

    // purchase
    let req = NewJobRequest {
        agent_id: AgentId("agent:tx1".into()),
        service_id: ServiceId("svc:en_ko".into()),
        input: serde_json::json!({"text":"hi"}),
        max_price_usdc_micro: None, // 마켓 정가 lookup
    };
    let result = tools.purchase(req).await.unwrap();
    match &result.decision {
        PurchaseDecision::AutoApproved => {}
        other => panic!("expected AutoApproved, got {other:?}"),
    }
    assert_eq!(result.amount_usdc_micro, 100_000);
    assert!(result.receipt.is_some());
    assert!(result.job.is_some());
    assert_eq!(gw.count(), 1);
    assert_eq!(gw.total_spent_micro(), 100_000);

    // get_job_status
    let job = tools.get_job_status("job:001").await.unwrap();
    assert_eq!(job.status.as_str(), "completed");

    m_search.assert_async().await;
    m_get.assert_async().await;
    m_post.assert_async().await;
    m_status.assert_async().await;
}

#[tokio::test]
async fn purchase_over_per_tx_limit_requires_confirmation() {
    let server = mockito::Server::new_async().await;

    // max_price만 명시 (마켓 lookup 안함) — 한도(0.5 USDC) 초과
    let mut policy = SpendPolicy::default();
    policy.allow(&AgentId("agent:expensive".into()));
    let (tools, gw) = make_tools(&server.url(), policy);

    let req = NewJobRequest {
        agent_id: AgentId("agent:expensive".into()),
        service_id: ServiceId("svc:big".into()),
        input: serde_json::Value::Null,
        max_price_usdc_micro: Some(2_000_000), // $2 > $0.50 per-tx
    };
    let result = tools.purchase(req).await.unwrap();
    match &result.decision {
        PurchaseDecision::NeedsConfirmation { reason } => {
            assert!(reason.contains("per-tx"));
        }
        other => panic!("expected NeedsConfirmation, got {other:?}"),
    }
    // 결제도 안 했고 job도 안 만듦
    assert_eq!(gw.count(), 0);
    assert!(result.receipt.is_none());
    assert!(result.job.is_none());
}

#[tokio::test]
async fn purchase_without_whitelist_requires_confirmation() {
    let server = mockito::Server::new_async().await;

    let policy = SpendPolicy::default(); // whitelist 비어있음, whitelist_required = true
    let (tools, gw) = make_tools(&server.url(), policy);

    let req = NewJobRequest {
        agent_id: AgentId("agent:stranger".into()),
        service_id: ServiceId("svc:x".into()),
        input: serde_json::Value::Null,
        max_price_usdc_micro: Some(100_000), // 한도 내
    };
    let result = tools.purchase(req).await.unwrap();
    match &result.decision {
        PurchaseDecision::NeedsConfirmation { reason } => {
            assert!(reason.contains("whitelist"));
        }
        other => panic!("expected NeedsConfirmation, got {other:?}"),
    }
    assert_eq!(gw.count(), 0);
}

#[tokio::test]
async fn search_handles_bare_array_response() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/agents")
        .match_query(mockito::Matcher::AnyOf(vec![
            mockito::Matcher::UrlEncoded("q".into(), "code".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"id":"agent:a","name":"A","description":"x"}]"#)
        .create_async()
        .await;

    let (tools, _) = make_tools(&server.url(), SpendPolicy::default());
    let res = tools.search("code", None).await.unwrap();
    assert_eq!(res.count, 1);
    m.assert_async().await;
}

#[tokio::test]
async fn get_agent_with_bare_object_response() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/agents/agent:bare")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id":"agent:bare","name":"Bare","description":"x"}"#)
        .create_async()
        .await;
    let (tools, _) = make_tools(&server.url(), SpendPolicy::default());
    let a = tools.get_agent("agent:bare").await.unwrap();
    assert_eq!(a.id.as_str(), "agent:bare");
    m.assert_async().await;
}

#[tokio::test]
async fn get_job_status_http_404_is_error() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/jobs/job:missing")
        .with_status(404)
        .with_body(r#"{"error":"not found"}"#)
        .create_async()
        .await;
    let (tools, _) = make_tools(&server.url(), SpendPolicy::default());
    let res = tools.get_job_status("job:missing").await;
    assert!(res.is_err());
    m.assert_async().await;
}

#[tokio::test]
async fn empty_agent_id_rejected_at_get_agent() {
    let server = mockito::Server::new_async().await;
    let (tools, _) = make_tools(&server.url(), SpendPolicy::default());
    let res = tools.get_agent("").await;
    assert!(res.is_err());
}
