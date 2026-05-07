//! xgram payment — payment intent CLI (PRD §16 baseline).
//!
//! 결제 인프라 만 — 실제 on-chain 제출은 후속 PR.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::env::require_password;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_payment::{chain, submit_intent, PaymentState, PaymentStore};

#[derive(Debug, Clone)]
pub enum PaymentAction {
    New {
        amount_usdc: String, // "1.50" — micro 변환은 내부에서
        chain: String,
        to: String,
        memo: Option<String>,
    },
    Sign {
        id: String,
    },
    List,
    Show {
        id: String,
    },
    Chains,
    MarkSubmitted {
        id: String,
        tx_hash: String,
    },
    MarkConfirmed {
        id: String,
    },
    MarkFailed {
        id: String,
        reason: String,
    },
}

/// `payment submit` — signed intent 를 RPC 로 on-chain 제출.
/// rpc_url 이 None 이면 chain 의 default_rpc 사용 (env override 가능).
/// notify_peer_alias 가 있으면 송금 성공 직후 해당 peer 에게 결제 통지 envelope 발송.
pub async fn run_payment_submit(
    data_dir: &Path,
    id: &str,
    rpc_url: Option<&str>,
    notify_peer_alias: Option<&str>,
) -> Result<()> {
    let pw = openxgram_core::env::require_password()?;
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let master = ks
        .load(MASTER_KEY_NAME, &pw)
        .context("master 키 로드 실패")?;

    // db/store 는 scope 안에서만 사용 → block 종료 시 자동 drop.
    // peer_send 가 자체 db open 하므로, 그 전에 우리 핸들이 release 되어 있어야 한다.
    let (intent, tx_hash) = {
        let mut db = open_db(data_dir)?;
        let mut store = PaymentStore::new(&mut db);
        let intent = store
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("payment 없음: {id}"))?;
        if intent.state != PaymentState::Signed {
            bail!(
                "state != signed (현재: {}). `xgram payment sign {id}` 먼저 실행.",
                intent.state.as_str()
            );
        }

        let rpc = resolve_rpc_url(&intent.chain, rpc_url)?;
        println!("→ {} 로 RPC 제출 ({})", rpc, intent.amount_display());

        let tx_hash = submit_intent(&intent, &master, &rpc)
            .await
            .context("on-chain submit 실패")?;
        store.mark_submitted(id, &tx_hash)?;

        println!("✓ payment submit 성공");
        println!("  id      : {}", intent.id);
        println!("  amount  : {}", intent.amount_display());
        println!("  to      : {}", intent.payee_address);
        println!("  chain   : {}", intent.chain);
        println!("  tx_hash : {tx_hash}");
        (intent, tx_hash)
    };

    if let Some(alias) = notify_peer_alias {
        let body = build_payment_receipt_body(&intent, &tx_hash, &master.address.0);
        println!();
        println!("→ peer '{alias}' 에 결제 통지 발송 중...");
        crate::peer_send::run_peer_send(data_dir, alias, None, &body, &pw)
            .await
            .with_context(|| format!("peer '{alias}' 통지 실패"))?;
        println!("✓ 통지 완료 — 수취인 inbox 에 결제 영수증 기록.");
    }

    println!();
    println!("확정 모니터링: 별도 watcher 또는 `xgram payment mark-confirmed {id}` (수동).");
    Ok(())
}

/// 수취인이 inbox 에서 식별 가능하도록 magic prefix + JSON 포맷.
/// 첫 줄은 고정 식별자 "xgr-payment-receipt-v1" — 향후 receiver 측 구조화 파서 진입점.
fn build_payment_receipt_body(
    intent: &openxgram_payment::PaymentIntent,
    tx_hash: &str,
    sender_address: &str,
) -> String {
    let json = serde_json::json!({
        "intent_id": intent.id,
        "tx_hash": tx_hash,
        "chain": intent.chain,
        "amount_usdc_micro": intent.amount_usdc_micro,
        "amount_display": intent.amount_display(),
        "to": intent.payee_address,
        "from": sender_address,
        "memo": intent.memo,
        "nonce": intent.nonce,
    });
    format!("xgr-payment-receipt-v1\n{json}")
}

fn resolve_rpc_url(chain_name: &str, override_url: Option<&str>) -> Result<String> {
    if let Some(u) = override_url {
        if !u.trim().is_empty() {
            return Ok(u.to_string());
        }
    }
    // chain 별 env 우선
    let env_key = match chain_name {
        "base" => "XGRAM_BASE_RPC_PRIMARY",
        "base-sepolia" => "XGRAM_BASE_SEPOLIA_RPC",
        _ => "",
    };
    if !env_key.is_empty() {
        if let Ok(v) = std::env::var(env_key) {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
    }
    let cfg = chain::lookup(chain_name)
        .ok_or_else(|| anyhow::anyhow!("지원하지 않는 chain: {chain_name}"))?;
    Ok(cfg.default_rpc.to_string())
}

pub fn run_payment(data_dir: &Path, action: PaymentAction) -> Result<()> {
    // Chains 명령은 db 미존재 환경에서도 작동 (메모리만)
    if let PaymentAction::Chains = action {
        println!("지원 chain ({}):", chain::ALL.len());
        for c in chain::ALL {
            println!(
                "  {} (chain_id={}, USDC={})",
                c.name, c.chain_id, c.usdc_contract
            );
            println!("    default RPC: {}", c.default_rpc);
        }
        return Ok(());
    }

    let mut db = open_db(data_dir)?;
    let mut store = PaymentStore::new(&mut db);

    match action {
        PaymentAction::Chains => unreachable!("handled above"),
        PaymentAction::New {
            amount_usdc,
            chain: chain_name,
            to,
            memo,
        } => {
            if chain::lookup(&chain_name).is_none() {
                bail!("지원하지 않는 chain: {chain_name} — `xgram payment chains` 로 목록 확인");
            }
            let micro = parse_usdc_amount(&amount_usdc)?;
            let intent = store.create_draft(micro, &chain_name, &to, memo.as_deref())?;
            println!("✓ payment intent draft 생성");
            println!("  id     : {}", intent.id);
            println!("  amount : {}", intent.amount_display());
            println!("  chain  : {}", intent.chain);
            println!("  to     : {}", intent.payee_address);
            println!("  nonce  : {}", intent.nonce);
            println!("  state  : {}", intent.state.as_str());
            println!();
            println!("서명: xgram payment sign {}", intent.id);
        }
        PaymentAction::Sign { id } => {
            let pw = require_password()?;
            let ks = FsKeystore::new(keystore_dir(data_dir));
            let master = ks
                .load(MASTER_KEY_NAME, &pw)
                .context("master 키 로드 실패")?;
            let signed = store.sign(&id, &master)?;
            println!("✓ payment intent 서명");
            println!("  id        : {}", signed.id);
            println!("  state     : {}", signed.state.as_str());
            let sig = signed.signature_hex.unwrap_or_default();
            if !sig.is_empty() {
                println!("  signature : {}…{}", &sig[..16], &sig[sig.len() - 16..]);
            }
        }
        PaymentAction::List => {
            let intents = store.list()?;
            if intents.is_empty() {
                println!("payment intents 없음.");
                return Ok(());
            }
            println!("payment intents ({})", intents.len());
            for i in &intents {
                println!(
                    "  {} — {} → {} [{}] ({})",
                    i.id,
                    i.amount_display(),
                    i.payee_address,
                    i.chain,
                    i.state.as_str(),
                );
            }
        }
        PaymentAction::Show { id } => {
            let intent = store
                .get(&id)?
                .ok_or_else(|| anyhow::anyhow!("payment 없음: {id}"))?;
            println!("payment {}", intent.id);
            println!("  amount     : {}", intent.amount_display());
            println!("  chain      : {}", intent.chain);
            println!("  to         : {}", intent.payee_address);
            println!("  nonce      : {}", intent.nonce);
            println!("  state      : {}", intent.state.as_str());
            println!("  created_at : {}", intent.created_at);
            if let Some(sa) = intent.signed_at {
                println!("  signed_at  : {sa}");
            }
            if let Some(memo) = &intent.memo {
                println!("  memo       : {memo}");
            }
            if let Some(tx) = &intent.submitted_tx_hash {
                println!("  tx_hash    : {tx}");
            }
            if let Some(reason) = &intent.error_reason {
                println!("  error      : {reason}");
            }
        }
        PaymentAction::MarkSubmitted { id, tx_hash } => {
            store.mark_submitted(&id, &tx_hash)?;
            println!("✓ payment {id} → submitted (tx={tx_hash})");
        }
        PaymentAction::MarkConfirmed { id } => {
            store.mark_confirmed(&id)?;
            println!("✓ payment {id} → confirmed");
        }
        PaymentAction::MarkFailed { id, reason } => {
            store.mark_failed(&id, &reason)?;
            println!("✓ payment {id} → failed ({reason})");
        }
    }
    Ok(())
}

/// "1.50" / "0.001" / "10" → micro USDC. 6 decimals 초과 입력은 raise.
fn parse_usdc_amount(s: &str) -> Result<i64> {
    let s = s.trim();
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    let whole: i64 = whole
        .parse()
        .map_err(|e| anyhow::anyhow!("amount 정수부 파싱 실패: {e}"))?;
    if frac.len() > 6 {
        bail!("USDC 는 6 decimals — 입력 '{s}' 의 소수부 너무 김");
    }
    let frac_padded = format!("{frac:0<6}"); // 좌측 정렬, 6자리 padding (right pad with 0)
    let frac: i64 = if frac.is_empty() {
        0
    } else {
        frac_padded[..6]
            .parse()
            .map_err(|e| anyhow::anyhow!("amount 소수부 파싱 실패: {e}"))?
    };
    Ok(whole * 1_000_000 + frac)
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!("DB 미존재 ({}). `xgram init` 먼저 실행.", path.display());
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amount_basic() {
        assert_eq!(parse_usdc_amount("1").unwrap(), 1_000_000);
        assert_eq!(parse_usdc_amount("1.5").unwrap(), 1_500_000);
        assert_eq!(parse_usdc_amount("0.001").unwrap(), 1_000);
        assert_eq!(parse_usdc_amount("10.123456").unwrap(), 10_123_456);
        assert_eq!(parse_usdc_amount("0").unwrap(), 0);
    }

    #[test]
    fn parse_amount_too_many_decimals_rejected() {
        assert!(parse_usdc_amount("1.1234567").is_err());
    }
}
