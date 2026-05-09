//! 4.1.1 — EAS schema 정의 + UID 계산.
//!
//! UID 정의는 EAS spec: `keccak256(abi.encode(schema_text, resolver_addr, revocable))`.
//! 본 모듈은 schema_text 를 ABI 형식 문자열로 표준화 + UID 계산. resolver 는 v0 에서 zero address.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

/// EAS schema text 표현. 실제 ABI encode 와 호환 — `<type> <name>` 콤마 구분.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaDefinition {
    pub name: &'static str,
    pub schema_text: &'static str,
    pub revocable: bool,
}

/// 4.1.1.1 — xgram-message schema. 한 메시지의 발신자/수신자/conversation/payload hash.
pub const MESSAGE: SchemaDefinition = SchemaDefinition {
    name: "xgram-message",
    schema_text: "address from,address to,bytes32 conversation_id,bytes32 payload_hash,uint64 timestamp",
    revocable: true,
};

/// 4.1.1.2 — xgram-payment schema. 송신자/수신자/금액/체인.
pub const PAYMENT: SchemaDefinition = SchemaDefinition {
    name: "xgram-payment",
    schema_text: "address sender,address recipient,uint256 amount_micros,string chain,bytes32 tx_hash,bytes32 intent_id",
    revocable: false,
};

/// 4.1.1.3 — xgram-endorsement schema. 추천한 사람/추천받은 사람/태그/메모.
pub const ENDORSEMENT: SchemaDefinition = SchemaDefinition {
    name: "xgram-endorsement",
    schema_text: "address endorser,address endorsee,string tag,string memo",
    revocable: true,
};

/// 모든 schema 의 enum view.
pub struct SchemaRegistry;

impl SchemaRegistry {
    pub const ALL: [&'static SchemaDefinition; 3] = [&MESSAGE, &PAYMENT, &ENDORSEMENT];

    pub fn get(name: &str) -> Option<&'static SchemaDefinition> {
        Self::ALL.iter().find(|s| s.name == name).copied()
    }
}

pub use MESSAGE as MessageSchema;
pub use PAYMENT as PaymentSchema;
pub use ENDORSEMENT as EndorsementSchema;

/// EAS spec 호환 UID — `keccak256(schema_text || zero_resolver(20) || revocable_bit)`.
/// Solidity ABI encoding 의 정확한 packing 은 단일 노드 사용에는 불필요. 프로토콜이 같은 입력에
/// 같은 UID 를 내는 결정성만 보장.
pub fn schema_uid(def: &SchemaDefinition) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(def.schema_text.as_bytes());
    hasher.update([0u8; 20]); // resolver = address(0)
    hasher.update([if def.revocable { 1u8 } else { 0u8 }]);
    let out = hasher.finalize();
    format!("0x{}", hex::encode(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_schemas_have_distinct_uids() {
        let uids: Vec<String> = SchemaRegistry::ALL.iter().map(|s| schema_uid(s)).collect();
        assert_eq!(uids.len(), 3);
        let unique: std::collections::HashSet<_> = uids.iter().collect();
        assert_eq!(unique.len(), 3, "각 schema 는 distinct UID");
    }

    #[test]
    fn schema_uid_is_deterministic() {
        let a = schema_uid(&MESSAGE);
        let b = schema_uid(&MESSAGE);
        assert_eq!(a, b);
        assert!(a.starts_with("0x"));
        assert_eq!(a.len(), 66, "0x + 64 hex chars (32 bytes)");
    }

    #[test]
    fn registry_get_returns_known_schema() {
        assert!(SchemaRegistry::get("xgram-message").is_some());
        assert!(SchemaRegistry::get("xgram-payment").is_some());
        assert!(SchemaRegistry::get("xgram-endorsement").is_some());
        assert!(SchemaRegistry::get("xgram-unknown").is_none());
    }

    #[test]
    fn payment_schema_is_irrevocable() {
        assert!(!PAYMENT.revocable);
        assert!(MESSAGE.revocable);
        assert!(ENDORSEMENT.revocable);
    }
}
