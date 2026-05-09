//! 4.1.2 — 자동 attestation. 메시지·결제·endorsement 발생 시 호출되어 attestation row 를 만든다.
//!
//! offchain → 호출자가 schema 별 데이터 + 서명만 채워 store 에 저장. onchain 제출은 다음 단계.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::schema::{schema_uid, SchemaDefinition, SchemaRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttestationKind {
    Message,
    Payment,
    Endorsement,
}

impl AttestationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Payment => "payment",
            Self::Endorsement => "endorsement",
        }
    }
    pub fn schema(&self) -> &'static SchemaDefinition {
        match self {
            Self::Message => SchemaRegistry::get("xgram-message").unwrap(),
            Self::Payment => SchemaRegistry::get("xgram-payment").unwrap(),
            Self::Endorsement => SchemaRegistry::get("xgram-endorsement").unwrap(),
        }
    }
}

/// payload 의 schema 별 필드를 JSON 으로 직렬화 — offchain 검증 + 다음 단계 onchain encode 용 입력.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationData {
    pub kind: AttestationKind,
    pub fields: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Attestation {
    pub id: String,
    pub schema_uid: String,
    pub kind: AttestationKind,
    pub fields_json: String,
    /// data_hash = keccak256(schema_uid || fields_json) — offchain 결정성.
    pub data_hash: String,
    pub created_at: DateTime<FixedOffset>,
    /// onchain 제출이 완료되면 tx_hash (옵션). 미제출은 None.
    pub tx_hash: Option<String>,
}

impl Attestation {
    pub fn new(data: AttestationData) -> Self {
        let schema = data.kind.schema();
        let uid = schema_uid(schema);
        let fields_json = serde_json::to_string(&data.fields).unwrap_or_else(|_| "{}".into());
        let mut hasher = Keccak256::new();
        hasher.update(uid.as_bytes());
        hasher.update(fields_json.as_bytes());
        let data_hash = format!("0x{}", hex::encode(hasher.finalize()));
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            schema_uid: uid,
            kind: data.kind,
            fields_json,
            data_hash,
            created_at: kst_now(),
            tx_hash: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attestation_has_deterministic_data_hash() {
        let d = AttestationData {
            kind: AttestationKind::Message,
            fields: json!({"from": "0xa", "to": "0xb", "conversation_id": "c-1"}),
        };
        let a1 = Attestation::new(d.clone());
        let a2 = Attestation::new(d);
        assert_eq!(a1.data_hash, a2.data_hash);
        assert_eq!(a1.schema_uid, a2.schema_uid);
        assert_ne!(a1.id, a2.id, "id 는 매 호출 unique");
    }

    #[test]
    fn different_kinds_have_different_uids() {
        let m = Attestation::new(AttestationData {
            kind: AttestationKind::Message,
            fields: json!({}),
        });
        let p = Attestation::new(AttestationData {
            kind: AttestationKind::Payment,
            fields: json!({}),
        });
        assert_ne!(m.schema_uid, p.schema_uid);
    }
}
