//! 패스워드 기반 임의 데이터 암호화 — Argon2id + ChaCha20-Poly1305.
//!
//! 사용처: cold backup, 추후 메모리/세션 export 등 패스워드로 보호되는
//! binary 페이로드. keystore V3 keyfile 형식과 별개의 단순 헤더 포맷.
//!
//! 포맷:
//!   magic(6) || salt(16) || nonce(12) || ciphertext+tag(N+16)
//!
//! magic = "OXBK01" (OpenXgram Blob v01).

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::rand_core::{OsRng, RngCore};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};

use crate::error::KeystoreError;

const MAGIC: &[u8; 6] = b"OXBK01";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN + NONCE_LEN;

/// 암호화. salt·nonce 는 OsRng 로 새로 생성.
pub fn encrypt_blob(password: &str, plaintext: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| KeystoreError::Crypto(format!("encrypt: {e}")))?;

    let mut blob = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// 복호화. magic 불일치, 잘못된 패스워드, 헤더 truncation 모두 raise.
pub fn decrypt_blob(password: &str, blob: &[u8]) -> Result<Vec<u8>, KeystoreError> {
    if blob.len() < HEADER_LEN {
        return Err(KeystoreError::Crypto(format!(
            "blob too short: {} bytes (expected at least {})",
            blob.len(),
            HEADER_LEN
        )));
    }
    if &blob[..MAGIC.len()] != MAGIC {
        return Err(KeystoreError::Crypto(
            "invalid magic (expected OXBK01)".into(),
        ));
    }
    let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce_bytes = &blob[MAGIC.len() + SALT_LEN..HEADER_LEN];
    let ciphertext = &blob[HEADER_LEN..];

    let key = derive_key(password, salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| KeystoreError::InvalidPassword)
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], KeystoreError> {
    // storage.rs 의 V3 keyfile 과 동일한 Argon2id 파라미터 (m=64MiB, t=3, p=1).
    let params = Params::new(65536, 3, 1, Some(32))
        .map_err(|e| KeystoreError::Crypto(format!("argon2 params: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| KeystoreError::Crypto(format!("argon2 derive: {e}")))?;
    Ok(out)
}
