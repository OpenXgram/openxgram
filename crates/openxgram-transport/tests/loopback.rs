//! transport loopback 통합 테스트 — 같은 프로세스 안에서 서버 + 클라이언트.

use chrono::{FixedOffset, Utc};
use openxgram_transport::{
    send_envelope, spawn_server, spawn_server_with_rate_limit, Envelope, TransportError,
};

fn sample_envelope() -> Envelope {
    Envelope {
        from: "0xAAAA".into(),
        to: "0xBBBB".into(),
        payload_hex: "deadbeef".into(),
        timestamp: Utc::now().with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()),
        signature_hex: "00".repeat(64),
        nonce: None,
    }
}

#[tokio::test]
async fn round_trip_one_message() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}", server.bound_addr);

    let env = sample_envelope();
    send_envelope(&url, &env).await.unwrap();

    let received = server.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0], env);
    server.shutdown();
}

#[tokio::test]
async fn round_trip_multiple_messages() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}", server.bound_addr);

    for i in 0..5 {
        let mut env = sample_envelope();
        env.payload_hex = format!("{i:02x}");
        send_envelope(&url, &env).await.unwrap();
    }

    let received = server.received();
    assert_eq!(received.len(), 5);
    for (i, env) in received.iter().enumerate() {
        assert_eq!(env.payload_hex, format!("{i:02x}"));
    }
    server.shutdown();
}

#[tokio::test]
async fn send_to_nonexistent_endpoint_raises() {
    // 사용 안 하는 포트 — connection refused
    let env = sample_envelope();
    let err = send_envelope("http://127.0.0.1:1", &env).await.unwrap_err();
    assert!(matches!(err, TransportError::Http(_)), "got {err:?}");
}

#[tokio::test]
async fn health_endpoint_returns_status_ok() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}/v1/health", server.bound_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    // 새 필드 — uptime / received_count 항상 존재
    assert!(body["uptime_seconds"].is_number());
    assert_eq!(body["received_count"], 0);
    // tailscale_state / tailscale_ipv4 — 환경에 따라 string 또는 null
    assert!(body["tailscale_state"].is_string() || body["tailscale_state"].is_null());
    assert!(body["tailscale_ipv4"].is_string() || body["tailscale_ipv4"].is_null());
    server.shutdown();
}

#[tokio::test]
async fn replay_nonce_rejected_on_duplicate() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}/v1/message", server.bound_addr);
    let client = reqwest::Client::new();
    let mut env = sample_envelope();
    env.from = "0xReplayTest".into();
    env.nonce = Some("nonce-1".into());
    // 1st — OK
    let resp = client.post(&url).json(&env).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    // 2nd 동일 nonce — 409 Conflict
    let resp = client.post(&url).json(&env).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 409);
    server.shutdown();
}

#[tokio::test]
async fn rate_limit_returns_429_after_threshold() {
    // 명시적 threshold — 프로세스 env var을 건드리면 병렬 테스트 race 발생
    let server = spawn_server_with_rate_limit("127.0.0.1:0".parse().unwrap(), 3)
        .await
        .unwrap();
    let url = format!("http://{}/v1/message", server.bound_addr);
    let client = reqwest::Client::new();
    let mut env = sample_envelope();
    env.from = "0xRateLimitTest".into();
    // 3 OK
    for _ in 0..3 {
        let r = client.post(&url).json(&env).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
    // 4번째 — 429
    let r = client.post(&url).json(&env).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 429);
    server.shutdown();
}

#[tokio::test]
async fn timestamp_too_old_rejected() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}/v1/message", server.bound_addr);
    let client = reqwest::Client::new();
    let mut env = sample_envelope();
    env.from = "0xOldTimestamp".into();
    // 10분 전 — 90초 윈도우 초과
    env.timestamp = (Utc::now() - chrono::Duration::minutes(10))
        .with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap());
    let r = client.post(&url).json(&env).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 408); // REQUEST_TIMEOUT
    server.shutdown();
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url = format!("http://{}/v1/metrics", server.bound_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("openxgram_uptime_seconds"));
    assert!(body.contains("openxgram_received_total 0"));
    assert!(body.contains("# TYPE openxgram_uptime_seconds gauge"));
    server.shutdown();
}

#[tokio::test]
async fn metrics_with_provider_appends_extra_text() {
    use openxgram_transport::spawn_server_with_metrics;
    let provider: openxgram_transport::MetricsProvider = std::sync::Arc::new(|| {
        "# HELP custom_test 1\n# TYPE custom_test gauge\ncustom_test 42\n".into()
    });
    let server = spawn_server_with_metrics("127.0.0.1:0".parse().unwrap(), Some(provider))
        .await
        .unwrap();
    let url = format!("http://{}/v1/metrics", server.bound_addr);
    let body = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("custom_test 42"));
    assert!(body.contains("openxgram_uptime_seconds")); // baseline 도 함께
    server.shutdown();
}

#[tokio::test]
async fn health_received_count_grows_with_messages() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    // 메시지 3개 보내기
    let env = sample_envelope();
    let post_url = format!("http://{}/v1/message", server.bound_addr);
    let client = reqwest::Client::new();
    for _ in 0..3 {
        client.post(&post_url).json(&env).send().await.unwrap();
    }
    // health 조회 → received_count = 3
    let health_url = format!("http://{}/v1/health", server.bound_addr);
    let body: serde_json::Value = reqwest::Client::new()
        .get(&health_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["received_count"], 3);
    server.shutdown();
}

#[tokio::test]
async fn send_to_wrong_path_raises_4xx() {
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    // 잘못된 경로 — 405 또는 404
    let url = format!("http://{}/v1/wrong-path/v1/message", server.bound_addr);
    let env = sample_envelope();
    let err = reqwest::Client::new()
        .post(&url)
        .json(&env)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();
    assert!(err.status().is_some());
    server.shutdown();
}
