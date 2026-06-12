//! 마켓 (c)갈래 — 내부 ledger 기반 PaymentGateway 구현체.
//!
//! `openxgram_marketplace::PaymentGateway` trait 의 **프로덕션 구현** (dependency
//! inversion: marketplace crate 는 trait 만 노출, 실제 결제는 여기서).
//!
//! ## 왜 내부 ledger 인가 (온체인 USDC vs 내부 원장)
//!
//! `openxgram_payment::submit_intent` 는 실제 on-chain USDC 송금을 지원하지만
//! **funded wallet(잔액 보유) + chain RPC URL** 이 필요하다 (자금 없으면 RPC reject).
//! 현 단계는 그 두 전제가 없으므로 **OpenXgram 내부 지갑 원장(sub_wallets +
//! wallet_ledger)** 으로 1차 구현한다.
//!
//! NoopGateway 와의 결정적 차이 (가짜 영수증 금지):
//!   - Noop: 잔액 검증 X, 무조건 `0x_test_*` 영수증 → 가짜.
//!   - Ledger: sub_wallets 에서 **실제 잔액 검증 → 부족하면 결제 실패(에러)**,
//!     충분하면 spent_micro 차감 + wallet_ledger 기록 (감사 추적).
//!     영수증 tx_hash 는 `ledger:<uuid>` (내부 ledger ref — on-chain 아님 명시).
//!
//! 온체인 전환 시: `pay()` 내부에서 vault 키 로드 → `submit_intent(rpc_url)` 호출 →
//! 반환 tx_hash 를 receipt 에 채우고 ledger tx_ref 를 실제 tx hash 로 교체.

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use openxgram_db::{Db, DbConfig};
use openxgram_marketplace::{AgentId, PaymentGateway, PaymentReceipt};

use crate::daemon_gui_wallets;

/// 내부 원장 결제 게이트웨이. 자체 DB 연결(같은 sqlite 파일) 보유.
///
/// WAL + busy_timeout 으로 daemon 의 다른 DB 연결과 동시 접근 안전.
pub struct LedgerPaymentGateway {
    db: Mutex<Db>,
}

impl LedgerPaymentGateway {
    /// 같은 데이터 디렉토리의 db.sqlite 에 별도 연결을 연다.
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        let mut db = Db::open(DbConfig {
            path: db_path,
            ..Default::default()
        })?;
        // 자기 자신도 migrate (wallet_ledger 등 존재 보장). idempotent.
        db.migrate()?;
        Ok(Self { db: Mutex::new(db) })
    }
}

#[async_trait]
impl PaymentGateway for LedgerPaymentGateway {
    async fn pay(
        &self,
        agent: &AgentId,
        amount_usdc_micro: i64,
        chain: &str,
        payee_address: &str,
        memo: Option<&str>,
    ) -> Result<PaymentReceipt, String> {
        let agent_id = agent.as_str().to_string();
        let intent_id = uuid::Uuid::new_v4().to_string();

        // 실제 잔액 검증 + 차감 + 원장 기록 (원자적). 부족하면 Err → 결제 실패.
        let tx_ref = {
            let mut db = self.db.lock().map_err(|e| format!("ledger db lock: {e}"))?;
            daemon_gui_wallets::debit_for_purchase(
                &mut db,
                &agent_id,
                amount_usdc_micro,
                chain,
                payee_address,
                &intent_id,
                memo,
            )?
        };

        Ok(PaymentReceipt {
            intent_id,
            amount_usdc_micro,
            chain: chain.to_string(),
            payee_address: payee_address.to_string(),
            // 내부 ledger ref (on-chain tx hash 아님 — 명시).
            tx_hash: Some(tx_ref),
            memo: memo.map(str::to_string),
        })
    }

    async fn spent_today_micro(&self) -> Result<i64, String> {
        let mut db = self.db.lock().map_err(|e| format!("ledger db lock: {e}"))?;
        // 전체 구매 누적(절대값). 일 단위 윈도우는 후속(현재 SpendPolicy 보수적 기본).
        let l = daemon_gui_wallets::list_ledger(&mut db, None, 1)
            .map_err(|e| format!("ledger sum: {e}"))?;
        Ok(l.total_purchase_micro)
    }
}
