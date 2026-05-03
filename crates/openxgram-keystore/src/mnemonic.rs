use bip39::{Language, Mnemonic as Bip39Mnemonic};
use zeroize::Zeroize;

use crate::error::KeystoreError;

/// 지원 언어
#[derive(Debug, Clone, Copy, Default)]
pub enum MnemonicLanguage {
    #[default]
    English,
}

impl From<MnemonicLanguage> for Language {
    fn from(lang: MnemonicLanguage) -> Self {
        match lang {
            MnemonicLanguage::English => Language::English,
        }
    }
}

/// BIP39 니모닉 래퍼 — Drop 시 zeroize
pub struct Mnemonic {
    inner: Bip39Mnemonic,
    /// phrase 캐시 (words() iterator에서 빌드)
    phrase_cache: String,
}

impl Mnemonic {
    fn build_phrase(m: &Bip39Mnemonic) -> String {
        m.words().collect::<Vec<_>>().join(" ")
    }

    /// 24단어 니모닉 신규 생성 (256비트 엔트로피, OsRng)
    pub fn generate(lang: MnemonicLanguage) -> Self {
        let mut rng = bip39::rand_core::OsRng;
        let inner = Bip39Mnemonic::generate_in_with(&mut rng, lang.into(), 24)
            .expect("24-word mnemonic generation must not fail");
        let phrase_cache = Self::build_phrase(&inner);
        Self {
            inner,
            phrase_cache,
        }
    }

    /// 니모닉 문구 임포트 및 검증
    pub fn from_phrase(phrase: &str) -> Result<Self, KeystoreError> {
        let inner = Bip39Mnemonic::parse_in_normalized(Language::English, phrase)
            .map_err(|e| KeystoreError::InvalidMnemonic(e.to_string()))?;
        let phrase_cache = Self::build_phrase(&inner);
        Ok(Self {
            inner,
            phrase_cache,
        })
    }

    /// 니모닉 → BIP39 시드 (512비트, passphrase 옵션)
    pub fn to_seed(&self, passphrase: &str) -> [u8; 64] {
        self.inner.to_seed(passphrase)
    }

    /// 니모닉 문구 문자열 반환
    pub fn phrase(&self) -> &str {
        &self.phrase_cache
    }

    /// 단어 수 반환
    pub fn word_count(&self) -> usize {
        self.inner.word_count()
    }
}

impl Drop for Mnemonic {
    fn drop(&mut self) {
        self.phrase_cache.zeroize();
    }
}

impl std::fmt::Debug for Mnemonic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mnemonic([REDACTED])")
    }
}
