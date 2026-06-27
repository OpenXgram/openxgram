//! 원격 mutation 인가 게이트 — cross-daemon A2A 명령(identity_update / a2a_command)의
//! 공통 보안 게이트. fix/identity-update-auth-gate (보안 핫픽스) + 3-4 재사용.
//!
//! 위협: identity_update / a2a_command envelope 은 원격이 보낸 신호로 **로컬 DB 를 변경**한다.
//! 인가 없이 처리하면 누구든 임의 alias 의 display_name/role 을 원격 변경할 수 있다(auth-bypass).
//!
//! 게이트 3중 (ALL pass 필수):
//!   1. 발신 peer 가 등록되어 있고(peer_opt Some),
//!   2. 그 peer 의 저장된 pubkey 로 서명검증 통과(verify_with_pubkey),
//!   3. 그 peer 의 eth 가 신뢰 발행자 allowlist(default-deny)에 포함.
//!
//! allowlist 진리원천 = env `XGRAM_TRUSTED_ISSUERS`(콤마/공백 구분 eth 주소 목록).
//! 정적 하드코딩 금지(원칙7) — 미설정이면 default-deny(빈 집합 → 전부 거부).

use std::collections::BTreeSet;

/// env 문자열(`XGRAM_TRUSTED_ISSUERS`)을 신뢰 발행자 eth 집합으로 파싱.
/// 콤마/공백/탭/개행 구분, trim, 빈 항목 제거, **소문자 정규화**(EIP-55 대소문자 차이 흡수).
pub fn parse_trusted_issuers(env_val: &str) -> BTreeSet<String> {
    env_val
        .split([',', ' ', '\t', '\n'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// 발행자 eth 가 신뢰 allowlist 에 있는지. default-deny: 빈 allowlist → 항상 false.
/// 비교는 소문자 정규화로 EIP-55 대소문자 무관. 빈 발행자는 거부.
pub fn is_trusted_issuer(issuer_eth: &str, allowlist: &BTreeSet<String>) -> bool {
    let key = issuer_eth.trim().to_ascii_lowercase();
    if key.is_empty() {
        return false;
    }
    allowlist.contains(&key)
}

/// 게이트가 신원검증에 쓰는 발신 peer 의 최소 신원 (DB peers row 에서 추출).
/// 전체 Peer 가 아니라 게이트가 실제 의존하는 두 필드만 받아 테스트·재사용을 단순화.
pub struct IssuerPeer {
    /// 수신측 DB 에 등록된 발신자 compressed secp256k1 pubkey hex (서명검증 기준).
    pub public_key_hex: String,
    /// 발신자 EIP-55 eth 주소 (allowlist 매칭 기준).
    pub eth_address: String,
}

/// 원격 mutation 인가 게이트 (3중, ALL pass).
/// 통과 시 신뢰 발행자 eth(소문자 정규화)를 Some 으로, 거부 시 None.
///
/// 1. `peer` Some — 발신자가 수신측 DB 에 등록된 peer 여야 함(미등록 → 거부).
/// 2. `verify_with_pubkey(peer.public_key_hex, payload, sig)` 통과 — **DB 에 저장된 pubkey** 로
///    대조하므로 "claim pubkey + 자기서명" 위장 불가.
/// 3. `peer.eth_address` ∈ allowlist(default-deny) — verified 라도 fleet 멤버가 아니면 거부.
pub fn authorize_remote_mutation(
    peer: Option<&IssuerPeer>,
    payload: &[u8],
    signature: &[u8],
    allowlist: &BTreeSet<String>,
) -> Option<String> {
    let peer = peer?; // gate 1: 등록 peer 필수
    // gate 2: 등록된 pubkey 로 서명검증
    if openxgram_keystore::verify_with_pubkey(&peer.public_key_hex, payload, signature).is_err() {
        return None;
    }
    // gate 3: 신뢰 allowlist 멤버
    if !is_trusted_issuer(&peer.eth_address, allowlist) {
        return None;
    }
    Some(peer.eth_address.trim().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_splits_comma_and_space_lowercases() {
        let set = parse_trusted_issuers("0xAbC, 0xDEF 0x123");
        assert_eq!(set.len(), 3);
        assert!(set.contains("0xabc"));
        assert!(set.contains("0xdef"));
        assert!(set.contains("0x123"));
    }

    #[test]
    fn parse_empty_yields_empty_set() {
        assert!(parse_trusted_issuers("").is_empty());
        assert!(parse_trusted_issuers("   ").is_empty());
    }

    #[test]
    fn trusted_issuer_in_allowlist_case_insensitive() {
        let allow = parse_trusted_issuers("0xAAA,0xBBB");
        // EIP-55 대소문자 달라도 매칭
        assert!(is_trusted_issuer("0xaAa", &allow));
        assert!(is_trusted_issuer("0XBBB", &allow));
    }

    #[test]
    fn default_deny_empty_allowlist_rejects_all() {
        let empty = parse_trusted_issuers("");
        assert!(!is_trusted_issuer("0xAAA", &empty));
    }

    #[test]
    fn unknown_issuer_rejected() {
        let allow = parse_trusted_issuers("0xAAA");
        assert!(!is_trusted_issuer("0xCCC", &allow));
    }

    #[test]
    fn blank_issuer_rejected() {
        let allow = parse_trusted_issuers("0xAAA");
        assert!(!is_trusted_issuer("", &allow));
        assert!(!is_trusted_issuer("   ", &allow));
    }

    // ── 통합 게이트 authorize_remote_mutation ──
    use openxgram_keystore::Keypair;

    /// 테스트용 발신자: 32바이트 시드로 keypair + pubkey_hex 생성.
    fn issuer(seed: u8) -> (Keypair, String) {
        let kp = Keypair::from_secret_bytes(&[seed; 32]).expect("keypair");
        let pubkey_hex = hex::encode(kp.public_key_bytes());
        (kp, pubkey_hex)
    }

    #[test]
    fn legit_fleet_issuer_authorized() {
        let (kp, pubkey_hex) = issuer(1);
        let eth = "0xFleetDaemon";
        let allow = parse_trusted_issuers(eth);
        let payload = b"{\"action\":\"set_role\"}";
        let sig = kp.sign(payload);
        let peer = Some(IssuerPeer { public_key_hex: pubkey_hex, eth_address: eth.to_string() });

        let got = authorize_remote_mutation(peer.as_ref(), payload, &sig, &allow);
        assert_eq!(got.as_deref(), Some("0xfleetdaemon"));
    }

    #[test]
    fn unregistered_peer_rejected() {
        let allow = parse_trusted_issuers("0xFleetDaemon");
        let got = authorize_remote_mutation(None, b"x", b"sig", &allow);
        assert!(got.is_none(), "미등록 peer 는 거부");
    }

    #[test]
    fn forged_signature_rejected() {
        // 발행자가 주장하는 pubkey 는 kp1 인데, 서명은 kp2(공격자) 키로 함 → 검증 실패.
        let (_kp1, pubkey_hex) = issuer(1);
        let (kp2, _) = issuer(2);
        let eth = "0xFleetDaemon";
        let allow = parse_trusted_issuers(eth);
        let payload = b"{\"action\":\"kill\"}";
        let forged_sig = kp2.sign(payload);
        let peer = Some(IssuerPeer { public_key_hex: pubkey_hex, eth_address: eth.to_string() });

        let got = authorize_remote_mutation(peer.as_ref(), payload, &forged_sig, &allow);
        assert!(got.is_none(), "서명 불일치 → 거부 (auth-bypass 차단)");
    }

    #[test]
    fn verified_but_not_in_allowlist_rejected() {
        // 서명은 정당하지만 그 eth 가 allowlist 에 없음 → 거부(자기키+자기서명 위장 차단).
        let (kp, pubkey_hex) = issuer(3);
        let payload = b"{\"action\":\"set_role\"}";
        let sig = kp.sign(payload);
        let allow = parse_trusted_issuers("0xSomeOtherFleet"); // 발행자 eth 미포함
        let peer = Some(IssuerPeer { public_key_hex: pubkey_hex, eth_address: "0xRandomVerified".to_string() });

        let got = authorize_remote_mutation(peer.as_ref(), payload, &sig, &allow);
        assert!(got.is_none(), "verified 라도 allowlist 외면 거부");
    }
}
