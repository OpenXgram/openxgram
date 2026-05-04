//! 통합 — 3-step chain 실행 시 sender 가 정확히 N번 호출되는지 검증.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    ChainDefinition, ChainRunner, ChainStore, ChannelSender, OrchestrationError,
};

struct MockSender {
    role_calls: Arc<AtomicUsize>,
    platform_calls: Arc<AtomicUsize>,
    response: String,
}

#[async_trait]
impl ChannelSender for MockSender {
    async fn send_to_role(
        &self,
        _role: &str,
        _payload: &str,
    ) -> std::result::Result<String, OrchestrationError> {
        self.role_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
    async fn send_to_platform(
        &self,
        _platform: &str,
        _channel_id: &str,
        _text: &str,
    ) -> std::result::Result<String, OrchestrationError> {
        self.platform_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

fn temp_db() -> (tempfile::NamedTempFile, Db) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().to_path_buf(),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    (tmp, db)
}

#[tokio::test]
async fn three_step_chain_calls_sender_three_times() {
    let (_tmp, mut db) = temp_db();
    let store = ChainStore::new(db.conn());
    let yaml = r#"
name: standup
description: morning standup
steps:
  - to_role: master
    text: "오늘 일정 어떠세요?"
    delay_secs: 0
  - to_role: res
    text: "오늘 뉴스 요약 부탁"
    delay_secs: 0
  - to_platform: discord
    channel_id: "12345"
    text: "✓ standup 시작"
    delay_secs: 0
"#;
    let def: ChainDefinition = serde_yaml::from_str(yaml).unwrap();
    store.create(&def).unwrap();
    let (chain, steps) = store.get_by_name("standup").unwrap();
    let role_calls = Arc::new(AtomicUsize::new(0));
    let platform_calls = Arc::new(AtomicUsize::new(0));
    let sender = MockSender {
        role_calls: role_calls.clone(),
        platform_calls: platform_calls.clone(),
        response: "ok".to_string(),
    };
    let result = ChainRunner::run(&steps, &sender, &chain.name).await;
    assert!(!result.failed);
    assert_eq!(role_calls.load(Ordering::SeqCst), 2);
    assert_eq!(platform_calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.steps.len(), 3);
    assert!(result.steps.iter().all(|s| s.executed));
}

#[tokio::test]
async fn condition_blocks_step() {
    let (_tmp, mut db) = temp_db();
    let store = ChainStore::new(db.conn());
    let yaml = r#"
name: cond
steps:
  - to_role: master
    text: "Q?"
  - to_platform: discord
    channel_id: "12345"
    text: "ok"
    condition_kind: response_contains
    condition_value: "ok"
"#;
    let def: ChainDefinition = serde_yaml::from_str(yaml).unwrap();
    store.create(&def).unwrap();
    let (chain, steps) = store.get_by_name("cond").unwrap();
    let role_calls = Arc::new(AtomicUsize::new(0));
    let platform_calls = Arc::new(AtomicUsize::new(0));
    // master 응답이 "ok" 포함 → discord step 실행
    let sender = MockSender {
        role_calls: role_calls.clone(),
        platform_calls: platform_calls.clone(),
        response: "all ok!".to_string(),
    };
    let _ = ChainRunner::run(&steps, &sender, &chain.name).await;
    assert_eq!(platform_calls.load(Ordering::SeqCst), 1);
}
