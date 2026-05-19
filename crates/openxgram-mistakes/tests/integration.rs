//! 통합 시나리오 — log → check → resolve end-to-end.

use openxgram_mistakes::{MistakeTools, NewMistake};
use rusqlite::Connection;

fn fresh_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../openxgram-db/migrations/0019_mistakes.sql"
    ))
    .unwrap();
    conn
}

#[test]
fn end_to_end_log_check_resolve() {
    let conn = fresh_db();
    let tools = MistakeTools::new(&conn);

    // 1. 실수 등록
    let logged = tools
        .log(NewMistake {
            session_id: "session:dev".into(),
            intended_action: "git push --force to main".into(),
            actual_outcome: "팀원 작업 3개 commit 손실".into(),
            failure_reason: "force push 전 동료 commit 확인 안 함".into(),
            lesson: "main에는 절대 force push 금지. PR + review 필수.".into(),
            severity: Some(9),
            related_wiki: Some("git_workflow".into()),
        })
        .unwrap();
    assert_eq!(logged.severity, 9);

    // 2. 같은 의도 행동 계획 → check 호출 (LIKE substring 매칭)
    let check = tools.check("git push --force", 5).unwrap();
    assert_eq!(check.similar_count, 1);
    assert!(check.warnings[0].contains("force push"));

    // 3. find_similar
    let similar = tools.find_similar("commit 손실", 5).unwrap();
    assert_eq!(similar.len(), 1);

    // 4. resolve
    tools
        .resolve(&logged.id, "GitHub branch protection rule 활성 + pre-receive hook")
        .unwrap();

    // 5. 해결됨 확인
    let after = tools.find_similar("force push", 5).unwrap();
    assert!(after[0].resolved);
}

#[test]
fn check_with_no_matches_returns_empty() {
    let conn = fresh_db();
    let tools = MistakeTools::new(&conn);
    let res = tools.check("완전히 새로운 행동", 5).unwrap();
    assert_eq!(res.similar_count, 0);
    assert!(res.warnings.is_empty());
}

#[test]
fn severity_default_when_not_specified() {
    let conn = fresh_db();
    let tools = MistakeTools::new(&conn);
    let logged = tools
        .log(NewMistake {
            session_id: "session:s".into(),
            intended_action: "x".into(),
            actual_outcome: "y".into(),
            failure_reason: "z".into(),
            lesson: "w".into(),
            severity: None,
            related_wiki: None,
        })
        .unwrap();
    assert_eq!(logged.severity, 5);
}
