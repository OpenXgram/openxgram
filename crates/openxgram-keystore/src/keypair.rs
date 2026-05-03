use k256::{
    ecdsa::{signature::Signer, signature::Verifier, Signature, SigningKey, VerifyingKey},
    SecretKey,
};
use sha3::{Digest, Keccak256};
use zeroize::Zeroize;

use crate::error::KeystoreError;

/// EVM 호환 에이전트 주소 (EIP-55 체크섬 형식)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentAddress(pub String);

impl AgentAddress {
    /// secp256k1 공개키에서 Keccak256 기반 EVM 주소 계산 (EIP-55 체크섬)
    pub fn from_verifying_key(vk: &VerifyingKey) -> Self {
        let encoded = vk.to_encoded_point(false);
        // 압축 해제된 공개키: 04 || x (32) || y (32) — 앞 1바이트(04) 제거
        let pubkey_bytes = &encoded.as_bytes()[1..];
        let hash = Keccak256::digest(pubkey_bytes);
        // 마지막 20바이트
        let addr_bytes = &hash[12..];
        let addr_hex = hex::encode(addr_bytes);
        let checksummed = eip55_checksum(&addr_hex);
        AgentAddress(format!("0x{checksummed}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// EIP-55 체크섬 인코딩
fn eip55_checksum(hex_addr: &str) -> String {
    let addr_lower = hex_addr.to_lowercase();
    let hash = Keccak256::digest(addr_lower.as_bytes());
    addr_lower
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c.is_ascii_digit() {
                c
            } else {
                let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0xf;
                if nibble >= 8 {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            }
        })
        .collect()
}

/// secp256k1 키페어 — Drop 시 비밀키 zeroize
pub struct Keypair {
    signing_key: SigningKey,
    pub address: AgentAddress,
}

impl Keypair {
    /// SecretKey 바이트 슬라이스에서 생성
    pub fn from_secret_bytes(secret: &[u8]) -> Result<Self, KeystoreError> {
        let sk = SecretKey::from_slice(secret)
            .map_err(|e| KeystoreError::Crypto(format!("invalid secret key: {e}")))?;
        let signing_key = SigningKey::from(sk);
        let vk = signing_key.verifying_key();
        let address = AgentAddress::from_verifying_key(vk);
        Ok(Self {
            signing_key,
            address,
        })
    }

    /// 공개키(압축 33바이트) 반환
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec()
    }

    /// 비밀키 바이트 반환 — 주의: 호출자가 zeroize 책임
    pub fn secret_key_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    /// 메시지에 ECDSA 서명
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes().to_vec()
    }

    /// ECDSA 서명 검증
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), KeystoreError> {
        let sig =
            Signature::from_slice(signature).map_err(|_| KeystoreError::SignatureVerification)?;
        self.signing_key
            .verifying_key()
            .verify(message, &sig)
            .map_err(|_| KeystoreError::SignatureVerification)
    }
}

impl Drop for Keypair {
    fn drop(&mut self) {
        // signing_key 내부 바이트를 명시적으로 덮어쓰기
        let mut bytes = self.signing_key.to_bytes();
        bytes.zeroize();
    }
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keypair")
            .field("address", &self.address)
            .field("secret_key", &"[REDACTED]")
            .finish()
    }
}
