//! 마켓 (c)갈래 — 내부 지갑 원장 결제(차감) 통합 테스트.
//!
//! lib 내부 #[cfg(test)] 유닛은 별개의 pre-existing 컴파일 에러
//! (orchestration_adapter dyn Adapter: !Debug) 때문에 실행 불가하므로,
//! 결제 차감 로직만 격리 검증하는 integration test (별도 컴파일 단위).

use openxgram_cli::daemon_gui_wallets as w;
use openxgram_db::{Db, DbConfig};

fn test_db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::open(DbConfig {
        path: dir.path().join("db.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    (dir, db)
}

#[test]
fn ledger_topup_debit_real_balance() {
    let (_dir, mut db) = test_db();

    // 1. 서브 지갑 생성 (잔액 0).
    let sw = w::create_sub_wallet(
        &mut db,
        w::CreateSubWalletBody {
            agent_id: "agent:test".into(),
            derivation_index: None,
            derived_address: None,
        },
    )
    .unwrap();
    assert_eq!(sw.balance_micro, 0);

    // 2. 충전 $5 → 잔액 5_000_000 + ledger 'topup'.
    let sw = w::topup(
        &mut db,
        w::TopupBody { agent_id: "agent:test".into(), amount_micro: 5_000_000 },
    )
    .unwrap();
    assert_eq!(sw.balance_micro, 5_000_000);
    let lg = w::list_ledger(&mut db, Some("agent:test"), 50).unwrap();
    assert_eq!(lg.total_topup_micro, 5_000_000);
    assert_eq!(lg.entries.len(), 1);
    assert_eq!(lg.entries[0].kind, "topup");

    // 3. 구매 $2 → 실제 차감 (가짜 영수증 아님). 잔액 3_000_000.
    let tx_ref = w::debit_for_purchase(
        &mut db,
        "agent:test",
        2_000_000,
        "base",
        "market:agent:test",
        "intent-1",
        Some("job:1"),
    )
    .unwrap();
    assert!(tx_ref.starts_with("ledger:"));
    let sw = w::list_ledger(&mut db, Some("agent:test"), 50).unwrap();
    assert_eq!(sw.total_purchase_micro, 2_000_000); // 절대값 집계
    assert_eq!(sw.entries.len(), 2);

    // 4. 잔액 초과 구매 → 실패 (no fake success). 상태 불변.
    let err = w::debit_for_purchase(
        &mut db, "agent:test", 10_000_000, "base", "market:x", "intent-2", None,
    )
    .unwrap_err();
    assert!(err.contains("잔액 부족"), "got: {err}");

    // 5. 미존재 지갑 구매 → 실패.
    let err = w::debit_for_purchase(
        &mut db, "agent:nope", 1_000_000, "base", "market:x", "intent-3", None,
    )
    .unwrap_err();
    assert!(err.contains("미존재"), "got: {err}");

    // 6. 수익 적립 → earned_micro 증가 + ledger 'earn'.
    w::credit_earning(&mut db, "agent:test", 500_000, Some("ext:user1"), Some("외부 사용"))
        .unwrap();
    let lg = w::list_ledger(&mut db, Some("agent:test"), 50).unwrap();
    assert_eq!(lg.total_earned_micro, 500_000);
}
