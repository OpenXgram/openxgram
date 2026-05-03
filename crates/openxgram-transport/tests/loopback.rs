//! transport loopback 통합 테스트 — 같은 프로세스 안에서 서버 + 클라이언트.

use chrono::{FixedOffset, Utc};
use openxgram_transport::{send_envelope, spawn_server, Envelope, TransportError};

fn sample_envelope() -> Envelope {
    Envelope {
        from: "0xAAAA".into(),
        to: "0xBBBB".into(),
        payload_hex: "deadbeef".into(),
        timestamp: Utc::now().with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()),
        signature_hex: "00".repeat(64),
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
