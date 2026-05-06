//! age multi-recipient backup (PRD-BAK-01).
//!
//! 정책:
//! - master X25519 + 비상 복구 2 recipient (에이전트 마스터키 X25519, 오프라인 종이키)
//! - PQ readiness: WrapEngine trait 으로 추상화 — Kyber/Dilithium 교체는 NIST FIPS 203/204 후
//! - 단일 패스워드 키 분실 = 영구 손실 회피

use std::io::{Read, Write};

use age::secrecy::SecretString;
use age::{Decryptor, Encryptor, Recipient};
use anyhow::{anyhow, Context, Result};

/// 추상화된 wrap engine — 후속 PQ hybrid 도입 시 교체.
pub trait WrapEngine {
    fn encrypt(&self, plaintext: &[u8], recipients: &[String]) -> Result<Vec<u8>>;
    fn decrypt_with_passphrase(&self, ciphertext: &[u8], passphrase: &str) -> Result<Vec<u8>>;
}

/// age recipient 기반 wrap.
pub struct AgeWrapEngine;

impl WrapEngine for AgeWrapEngine {
    fn encrypt(&self, plaintext: &[u8], recipients: &[String]) -> Result<Vec<u8>> {
        if recipients.is_empty() {
            return Err(anyhow!(
                "최소 1개 recipient 필요 — multi-recipient 권장 (master + 비상 2)"
            ));
        }
        let parsed: Vec<Box<dyn Recipient + Send>> = recipients
            .iter()
            .map(|s| {
                let r: age::x25519::Recipient = s
                    .parse()
                    .map_err(|e| anyhow!("recipient 파싱 실패 ({s}): {e}"))?;
                Ok(Box::new(r) as Box<dyn Recipient + Send>)
            })
            .collect::<Result<_>>()?;

        let encryptor =
            Encryptor::with_recipients(parsed.iter().map(|r| r.as_ref() as &dyn Recipient))
                .context("age Encryptor::with_recipients 실패")?;

        let mut buf = Vec::new();
        let mut writer = encryptor
            .wrap_output(&mut buf)
            .context("age writer 생성 실패")?;
        writer.write_all(plaintext).context("encrypt write 실패")?;
        writer.finish().context("encrypt finish 실패")?;
        Ok(buf)
    }

    fn decrypt_with_passphrase(&self, ciphertext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
        let decryptor = Decryptor::new(ciphertext).context("Decryptor::new 실패")?;
        let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .map_err(|e| anyhow!("decrypt 실패: {e}"))?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).context("decrypt read 실패")?;
        Ok(out)
    }
}

/// passphrase 기반 wrap (키 관리 단순화 시).
pub fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let recipient = age::scrypt::Recipient::new(SecretString::from(passphrase.to_string()));
    let encryptor = Encryptor::with_recipients(std::iter::once(&recipient as &dyn Recipient))
        .context("age scrypt Encryptor 실패")?;
    let mut buf = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut buf)
        .context("age writer 생성 실패")?;
    writer.write_all(plaintext).context("encrypt write 실패")?;
    writer.finish().context("encrypt finish 실패")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x25519_recipient_round_trip() {
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public();
        let plaintext = b"vault backup payload";

        let engine = AgeWrapEngine;
        let ct = engine.encrypt(plaintext, &[recipient.to_string()]).unwrap();
        assert_ne!(ct, plaintext);

        // identity 로 복호 (passphrase 와 다른 path) — direct decryptor
        let decryptor = Decryptor::new(ct.as_slice()).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn passphrase_round_trip() {
        let plaintext = b"super-secret-data";
        let pw = "correct horse battery staple";
        let ct = encrypt_with_passphrase(plaintext, pw).unwrap();
        let engine = AgeWrapEngine;
        let pt = engine.decrypt_with_passphrase(&ct, pw).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn passphrase_wrong_fails() {
        let plaintext = b"data";
        let ct = encrypt_with_passphrase(plaintext, "right").unwrap();
        let engine = AgeWrapEngine;
        let err = engine.decrypt_with_passphrase(&ct, "wrong");
        assert!(err.is_err());
    }

    #[test]
    fn empty_recipients_rejected() {
        let engine = AgeWrapEngine;
        let err = engine.encrypt(b"data", &[]);
        assert!(err.is_err());
    }

    #[test]
    fn multiple_recipients_all_can_decrypt() {
        let id_a = age::x25519::Identity::generate();
        let id_b = age::x25519::Identity::generate();
        let recipients = vec![id_a.to_public().to_string(), id_b.to_public().to_string()];
        let engine = AgeWrapEngine;
        let plaintext = b"shared-secret";
        let ct = engine.encrypt(plaintext, &recipients).unwrap();

        for id in [id_a, id_b] {
            let decryptor = Decryptor::new(ct.as_slice()).unwrap();
            let mut reader = decryptor
                .decrypt(std::iter::once(&id as &dyn age::Identity))
                .unwrap();
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            assert_eq!(out, plaintext, "두 identity 모두 복호 가능");
        }
    }
}
