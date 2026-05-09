//! d) HITL — Human-in-the-loop. 봇이 작업 중 사람 결정 필요 시 호출.
//!
//! 흐름:
//!   1. 봇 → `request_human_input(question, options?)` 호출
//!   2. 본 모듈이 message body 를 magic prefix 로 만들어 outbox-to-human session 에 저장
//!   3. agent.rs 의 forward 분기가 그 메시지를 등록된 채널 (Discord/Telegram) 로 push
//!   4. 사람 (마스터) 가 채널 또는 `xgram human respond <id> <answer>` 로 응답
//!   5. 응답 메시지가 inbox-from-human 세션에 저장 → 봇이 polling 으로 받아 작업 재개
//!
//! Magic prefix:
//!   `xgram-human-input-required-v1\n<request_id>\n<question>\n[options...]`
//! Response:
//!   `xgram-human-response-v1\n<request_id>\n<answer>`

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use std::path::Path;
use uuid::Uuid;

pub const HUMAN_REQUEST_PREFIX: &str = "xgram-human-input-required-v1";
pub const HUMAN_RESPONSE_PREFIX: &str = "xgram-human-response-v1";
pub const HUMAN_OUTBOX_SESSION: &str = "outbox-to-human";
pub const HUMAN_INBOX_SESSION: &str = "inbox-from-human";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanRequest {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
}

impl HumanRequest {
    pub fn new(question: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            question: question.into(),
            options,
        }
    }

    pub fn body(&self) -> String {
        let mut body = format!("{HUMAN_REQUEST_PREFIX}\n{}\n{}", self.id, self.question);
        for opt in &self.options {
            body.push('\n');
            body.push_str(opt);
        }
        body
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanResponse {
    pub request_id: String,
    pub answer: String,
}

impl HumanResponse {
    pub fn body(&self) -> String {
        format!("{HUMAN_RESPONSE_PREFIX}\n{}\n{}", self.request_id, self.answer)
    }
}

pub fn parse_human_request(body: &str) -> Option<HumanRequest> {
    let mut lines = body.lines();
    if lines.next()? != HUMAN_REQUEST_PREFIX {
        return None;
    }
    let id = lines.next()?.trim().to_string();
    let question = lines.next()?.trim().to_string();
    if id.is_empty() || question.is_empty() {
        return None;
    }
    let options: Vec<String> = lines.map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    Some(HumanRequest { id, question, options })
}

pub fn parse_human_response(body: &str) -> Option<HumanResponse> {
    let mut lines = body.lines();
    if lines.next()? != HUMAN_RESPONSE_PREFIX {
        return None;
    }
    let request_id = lines.next()?.trim().to_string();
    let answer = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if request_id.is_empty() || answer.is_empty() {
        return None;
    }
    Some(HumanResponse { request_id, answer })
}

/// 봇 코드에서 호출 — outbox-to-human session 에 magic prefix 메시지 저장.
/// agent.rs 의 forward 분기가 channel 로 push (별도 코드 — DB write 만 하면 자동).
pub fn request_human_input(
    data_dir: &Path,
    agent_alias: &str,
    question: &str,
    options: Vec<String>,
) -> Result<HumanRequest> {
    let req = HumanRequest::new(question, options);
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let session = SessionStore::new(&mut db)
        .ensure_by_title(HUMAN_OUTBOX_SESSION, "outbound")
        .context("HITL outbox session ensure")?;
    let embedder = default_embedder()?;
    MessageStore::new(&mut db, embedder.as_ref())
        .insert(&session.id, agent_alias, &req.body(), "hitl-request", None)
        .context("HITL request 저장")?;
    eprintln!("[hitl] 사람 입력 요청 ({}): {}", req.id, req.question);
    Ok(req)
}

/// `xgram human respond <id> <answer>` 또는 채널 메시지 처리에서 호출.
/// inbox-from-human session 에 응답 저장 → 봇이 polling 으로 받음.
pub fn respond_human(data_dir: &Path, request_id: &str, answer: &str) -> Result<()> {
    if request_id.trim().is_empty() {
        bail!("request_id 비어있음");
    }
    if answer.trim().is_empty() {
        bail!("answer 비어있음");
    }
    let resp = HumanResponse {
        request_id: request_id.into(),
        answer: answer.into(),
    };
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let session = SessionStore::new(&mut db)
        .ensure_by_title(HUMAN_INBOX_SESSION, "inbound")
        .context("HITL inbox session ensure")?;
    let embedder = default_embedder()?;
    MessageStore::new(&mut db, embedder.as_ref())
        .insert(&session.id, "human", &resp.body(), "hitl-response", None)
        .context("HITL response 저장")?;
    eprintln!("[hitl] 사람 응답 저장 ({}): {}", request_id, answer);
    Ok(())
}

/// 봇 자율 루프용 — request 후 응답 polling. 응답 도착 시 answer 반환, timeout 시 None.
/// `poll_interval_ms` 마다 inbox-from-human 의 매칭 응답 확인.
pub async fn await_human_response(
    data_dir: &Path,
    request_id: &str,
    timeout_secs: u64,
    poll_interval_ms: u64,
) -> Result<Option<String>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let interval = std::time::Duration::from_millis(poll_interval_ms.max(50));
    while std::time::Instant::now() < deadline {
        if let Some(answer) = check_response(data_dir, request_id)? {
            return Ok(Some(answer));
        }
        tokio::time::sleep(interval).await;
    }
    Ok(None)
}

/// inbox-from-human 의 응답 1회 polling — 매칭 시 answer 반환.
pub fn check_response(data_dir: &Path, request_id: &str) -> Result<Option<String>> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let inbox = match SessionStore::new(&mut db)
        .list()?
        .into_iter()
        .find(|s| s.title == HUMAN_INBOX_SESSION)
    {
        Some(s) => s,
        None => return Ok(None),
    };
    let embedder = default_embedder()?;
    let messages = MessageStore::new(&mut db, embedder.as_ref())
        .list_for_session(&inbox.id)?;
    for m in messages {
        if let Some(r) = parse_human_response(&m.body) {
            if r.request_id == request_id {
                return Ok(Some(r.answer));
            }
        }
    }
    Ok(None)
}

/// 미응답 HITL 요청 목록 — outbox 의 magic prefix 메시지 중 응답 (inbox) 안 도착한 것.
pub fn list_pending_requests(data_dir: &Path) -> Result<Vec<HumanRequest>> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let sessions = SessionStore::new(&mut db).list()?;
    let outbox = match sessions.iter().find(|s| s.title == HUMAN_OUTBOX_SESSION) {
        Some(s) => s.clone(),
        None => return Ok(vec![]),
    };
    let inbox = sessions.iter().find(|s| s.title == HUMAN_INBOX_SESSION).cloned();
    let embedder = default_embedder()?;
    let outbox_msgs = MessageStore::new(&mut db, embedder.as_ref())
        .list_for_session(&outbox.id)?;
    let answered_ids: std::collections::HashSet<String> = if let Some(inbox) = inbox {
        let inbox_msgs = MessageStore::new(&mut db, embedder.as_ref())
            .list_for_session(&inbox.id)?;
        inbox_msgs
            .iter()
            .filter_map(|m| parse_human_response(&m.body))
            .map(|r| r.request_id)
            .collect()
    } else {
        Default::default()
    };
    Ok(outbox_msgs
        .iter()
        .filter_map(|m| parse_human_request(&m.body))
        .filter(|r| !answered_ids.contains(&r.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_core::paths::db_path;
    use openxgram_core::paths::manifest_path;
    use tempfile::tempdir;

    fn open_test_db(dir: &Path) -> Db {
        // ensure data_dir tree + manifest
        std::fs::create_dir_all(dir).unwrap();
        let mp = manifest_path(dir);
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        if !mp.exists() {
            std::fs::write(&mp, "{}").unwrap();
        }
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db
    }

    #[test]
    fn request_round_trip_via_body() {
        let req = HumanRequest::new("승인하시겠습니까?", vec!["OK".into(), "취소".into()]);
        let body = req.body();
        let parsed = parse_human_request(&body).unwrap();
        assert_eq!(parsed.id, req.id);
        assert_eq!(parsed.question, "승인하시겠습니까?");
        assert_eq!(parsed.options, vec!["OK".to_string(), "취소".into()]);
    }

    #[test]
    fn response_round_trip_via_body() {
        let r = HumanResponse {
            request_id: "abc-123".into(),
            answer: "OK".into(),
        };
        let parsed = parse_human_response(&r.body()).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn parse_request_rejects_other_prefix() {
        assert!(parse_human_request("xgram-other\nid\nq").is_none());
        assert!(parse_human_request("xgram-human-input-required-v1\nid\n").is_none());
    }

    #[test]
    fn request_then_respond_flows_through_db() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let _db = open_test_db(dir);

        let req = request_human_input(dir, "@market-bot", "외주 $200 승인?", vec!["OK".into(), "거절".into()])
            .unwrap();
        // 응답 전 — pending 1개
        let pending_before = list_pending_requests(dir).unwrap();
        assert_eq!(pending_before.len(), 1);
        assert_eq!(pending_before[0].id, req.id);

        respond_human(dir, &req.id, "OK").unwrap();
        // 응답 후 — pending 0
        let pending_after = list_pending_requests(dir).unwrap();
        assert!(pending_after.is_empty());
    }

    #[tokio::test]
    async fn await_response_returns_when_human_answers() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let _db = open_test_db(&dir);

        let req = request_human_input(&dir, "@bot", "외주 승인?", vec![]).unwrap();
        let req_id = req.id.clone();
        let dir_for_responder = dir.clone();
        // 별도 task 가 200ms 후 응답
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            respond_human(&dir_for_responder, &req_id, "OK").unwrap();
        });

        let answer = await_human_response(&dir, &req.id, 5, 50).await.unwrap();
        assert_eq!(answer.as_deref(), Some("OK"));
    }

    #[tokio::test]
    async fn await_response_times_out_when_no_answer() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let _db = open_test_db(dir);
        let req = request_human_input(dir, "@bot", "?", vec![]).unwrap();
        let answer = await_human_response(dir, &req.id, 1, 100).await.unwrap();
        assert!(answer.is_none(), "1초 안에 응답 없으면 None");
    }

    #[test]
    fn respond_rejects_empty_args() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let _db = open_test_db(dir);
        assert!(respond_human(dir, "", "x").is_err());
        assert!(respond_human(dir, "id", "").is_err());
    }
}
