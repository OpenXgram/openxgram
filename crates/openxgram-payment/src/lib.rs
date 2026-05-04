//! openxgram-payment — payment intent 인프라 (PRD §16).
//!
//! **결제 인프라만** — 실제 on-chain 제출은 후속 PR (alloy/ethers RPC 통합).
//! 이번 PR 의 목적: 결제 의도(intent)를 type-safe 하게 표현·서명·DB 저장.
//!
//! USDC 는 6 decimals → `amount_usdc_micro` 는 micro USDC (1 USDC = 1_000_000).
//!
//! 흐름:
//!   1. create_draft  — intent row 생성 (nonce 자동, state='draft')
//!   2. sign          — master 키페어로 canonical bytes 서명, state='signed'
//!   3. (후속) submit — RPC 로 on-chain 트랜잭션 제출, state='submitted'
//!   4. (후속) confirm — block 확정 폴링, state='confirmed'
//!
//! 서명 입력(canonical bytes):
//!   "openxgram-payment-v1\n{chain}\n{payee}\n{amount_usdc_micro}\n{nonce}\n{memo}"

pub mod alloy_bridge;
pub mod chain;
pub mod erc20;
pub mod evm_nonce;
pub mod submit;

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbError};
use openxgram_keystore::Keypair;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PaymentError {
    #[error("db error: {0}")]
    Db(#[from] DbError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid amount: {0}")]
    InvalidAmount(String),

    #[error("invalid state transition: {from} → {to}")]
    InvalidTransition { from: String, to: String },

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("hex decode: {0}")]
    Hex(#[from] hex::FromHexError),

    #[error("ecdsa signature: {0}")]
    Signature(String),
}

pub type Result<T> = std::result::Result<T, PaymentError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentState {
    Draft,
    Signed,
    Submitted,
    Confirmed,
    Failed,
}

impl PaymentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Signed => "signed",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "draft" => Self::Draft,
            "signed" => Self::Signed,
            "submitted" => Self::Submitted,
            "confirmed" => Self::Confirmed,
            "failed" => Self::Failed,
            other => return Err(PaymentError::InvalidState(other.into())),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaymentIntent {
    pub id: String,
    pub amount_usdc_micro: i64,
    pub chain: String,
    pub payee_address: String,
    pub memo: Option<String>,
    pub nonce: String,
    pub signature_hex: Option<String>,
    pub state: PaymentState,
    pub created_at: DateTime<FixedOffset>,
    pub signed_at: Option<DateTime<FixedOffset>>,
    pub submitted_tx_hash: Option<String>,
    pub submitted_at: Option<DateTime<FixedOffset>>,
    pub confirmed_at: Option<DateTime<FixedOffset>>,
    pub error_reason: Option<String>,
}

impl PaymentIntent {
    /// 사람이 읽기 쉬운 USDC 표현 (예: "1.50 USDC").
    pub fn amount_display(&self) -> String {
        let micro = self.amount_usdc_micro;
        let whole = micro / 1_000_000;
        let frac = micro % 1_000_000;
        // 자릿수 자동 — trailing zero 제거
        let frac_str = format!("{frac:06}").trim_end_matches('0').to_string();
        if frac_str.is_empty() {
            format!("{whole} USDC")
        } else {
            format!("{whole}.{frac_str} USDC")
        }
    }

    /// 서명 입력 (canonical bytes) — 모든 머신·언어 동일 표현.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let memo = self.memo.as_deref().unwrap_or("");
        format!(
            "openxgram-payment-v1\n{}\n{}\n{}\n{}\n{memo}",
            self.chain, self.payee_address, self.amount_usdc_micro, self.nonce
        )
        .into_bytes()
    }
}

pub struct PaymentStore<'a> {
    db: &'a mut Db,
}

impl<'a> PaymentStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// 새 draft intent 생성. nonce 는 UUID, signature 는 None.
    pub fn create_draft(
        &mut self,
        amount_usdc_micro: i64,
        chain: &str,
        payee_address: &str,
        memo: Option<&str>,
    ) -> Result<PaymentIntent> {
        if amount_usdc_micro <= 0 {
            return Err(PaymentError::InvalidAmount(format!(
                "must be > 0 (got {amount_usdc_micro})"
            )));
        }
        let id = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO payment_intents
             (id, amount_usdc_micro, chain, payee_address, memo, nonce, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', ?7)",
            rusqlite::params![
                id,
                amount_usdc_micro,
                chain,
                payee_address,
                memo,
                nonce,
                now_rfc
            ],
        )?;
        self.get(&id)?
            .ok_or_else(|| PaymentError::NotFound(format!("just-inserted: {id}")))
    }

    /// draft → signed. master 키페어가 signing_bytes() 를 ECDSA 서명.
    pub fn sign(&mut self, id: &str, master: &Keypair) -> Result<PaymentIntent> {
        let intent = self
            .get(id)?
            .ok_or_else(|| PaymentError::NotFound(id.into()))?;
        if intent.state != PaymentState::Draft {
            return Err(PaymentError::InvalidTransition {
                from: intent.state.as_str().into(),
                to: "signed".into(),
            });
        }
        let sig = master.sign(&intent.signing_bytes());
        let sig_hex = hex::encode(sig);
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE payment_intents
             SET signature_hex = ?1, state = 'signed', signed_at = ?2
             WHERE id = ?3 AND state = 'draft'",
            rusqlite::params![sig_hex, now_rfc, id],
        )?;
        self.get(id)?
            .ok_or_else(|| PaymentError::NotFound(format!("post-sign: {id}")))
    }

    /// signed → submitted (RPC 통합 후 호출). tx_hash 는 0x... 형식.
    pub fn mark_submitted(&mut self, id: &str, tx_hash: &str) -> Result<()> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE payment_intents
             SET state = 'submitted', submitted_tx_hash = ?1, submitted_at = ?2
             WHERE id = ?3 AND state = 'signed'",
            rusqlite::params![tx_hash, now_rfc, id],
        )?;
        if affected != 1 {
            return Err(PaymentError::InvalidTransition {
                from: "(non-signed)".into(),
                to: "submitted".into(),
            });
        }
        Ok(())
    }

    /// submitted → confirmed.
    pub fn mark_confirmed(&mut self, id: &str) -> Result<()> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE payment_intents
             SET state = 'confirmed', confirmed_at = ?1
             WHERE id = ?2 AND state = 'submitted'",
            rusqlite::params![now_rfc, id],
        )?;
        if affected != 1 {
            return Err(PaymentError::InvalidTransition {
                from: "(non-submitted)".into(),
                to: "confirmed".into(),
            });
        }
        Ok(())
    }

    /// any state → failed (예: RPC 거부).
    pub fn mark_failed(&mut self, id: &str, reason: &str) -> Result<()> {
        let affected = self.db.conn().execute(
            "UPDATE payment_intents SET state = 'failed', error_reason = ?1 WHERE id = ?2",
            rusqlite::params![reason, id],
        )?;
        if affected != 1 {
            return Err(PaymentError::NotFound(id.into()));
        }
        Ok(())
    }

    pub fn get(&mut self, id: &str) -> Result<Option<PaymentIntent>> {
        Self::map_row(self.db.conn().query_row(
            "SELECT id, amount_usdc_micro, chain, payee_address, memo, nonce, signature_hex,
                    state, created_at, signed_at, submitted_tx_hash, submitted_at, confirmed_at, error_reason
             FROM payment_intents WHERE id = ?1",
            [id],
            row_to_intent_tuple,
        ))
    }

    pub fn list(&mut self) -> Result<Vec<PaymentIntent>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, amount_usdc_micro, chain, payee_address, memo, nonce, signature_hex,
                    state, created_at, signed_at, submitted_tx_hash, submitted_at, confirmed_at, error_reason
             FROM payment_intents ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_intent_tuple)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(tuple_to_intent(r?)?);
        }
        Ok(out)
    }

    fn map_row(result: rusqlite::Result<IntentRowTuple>) -> Result<Option<PaymentIntent>> {
        match result {
            Ok(t) => Ok(Some(tuple_to_intent(t)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

type IntentRowTuple = (
    String,         // id
    i64,            // amount_usdc_micro
    String,         // chain
    String,         // payee_address
    Option<String>, // memo
    String,         // nonce
    Option<String>, // signature_hex
    String,         // state
    String,         // created_at
    Option<String>, // signed_at
    Option<String>, // submitted_tx_hash
    Option<String>, // submitted_at
    Option<String>, // confirmed_at
    Option<String>, // error_reason
);

fn row_to_intent_tuple(r: &rusqlite::Row) -> rusqlite::Result<IntentRowTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
        r.get(13)?,
    ))
}

fn tuple_to_intent(t: IntentRowTuple) -> Result<PaymentIntent> {
    let (
        id,
        amount,
        chain,
        payee,
        memo,
        nonce,
        sig,
        state,
        created,
        signed,
        tx_hash,
        submitted,
        confirmed,
        err,
    ) = t;
    Ok(PaymentIntent {
        id,
        amount_usdc_micro: amount,
        chain,
        payee_address: payee,
        memo,
        nonce,
        signature_hex: sig,
        state: PaymentState::parse(&state)?,
        created_at: parse_ts(&created)?,
        signed_at: signed.as_deref().map(parse_ts).transpose()?,
        submitted_tx_hash: tx_hash,
        submitted_at: submitted.as_deref().map(parse_ts).transpose()?,
        confirmed_at: confirmed.as_deref().map(parse_ts).transpose()?,
        error_reason: err,
    })
}

fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|e| PaymentError::InvalidTimestamp(e.to_string()))
}
