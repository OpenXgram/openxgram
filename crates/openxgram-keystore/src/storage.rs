use std::path::PathBuf;

use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2, Params,
};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng as AeadOsRng},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::KeystoreError;
use crate::hd::{derive_keypair, DerivationPath};
use crate::keypair::{AgentAddress, Keypair};
use crate::mnemonic::{Mnemonic, MnemonicLanguage};

/// Keystore 트레이트
pub trait Keystore {
    /// 새 HD 키페어 생성 — 니모닉 반환 (호출자가 안전하게 보관)
    fn create(&self, name: &str, password: &str) -> Result<(AgentAddress, String), KeystoreError>;

    /// 니모닉으로 키 복원
    fn import(
        &self,
        name: &str,
        phrase: &str,
        password: &str,
    ) -> Result<AgentAddress, KeystoreError>;

    /// 이름으로 키 로드
    fn load(&self, name: &str, password: &str) -> Result<Keypair, KeystoreError>;

    /// 저장된 키 목록
    fn list(&self) -> Result<Vec<KeystoreEntry>, KeystoreError>;

    /// 키 삭제
    fn delete(&self, name: &str) -> Result<(), KeystoreError>;
}

/// 키스토어 엔트리 메타데이터
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreEntry {
    pub name: String,
    pub address: String,
    pub derivation_path: String,
    pub created_at: String,
}

/// V3 JSON 암호화 저장 포맷
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedKeyFile {
    version: u32,
    name: String,
    address: String,
    derivation_path: String,
    created_at: String,
    crypto: CryptoParams,
}

#[derive(Debug, Serialize, Deserialize)]
struct CryptoParams {
    cipher: String,
    ciphertext: String, // hex
    nonce: String,      // hex
    kdf: String,
    kdf_params: KdfParams,
    salt: String,
    mac: String, // hex — chacha20poly1305 tag
}

#[derive(Debug, Serialize, Deserialize)]
struct KdfParams {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    output_len: usize,
}

/// 파일시스템 기반 키스토어
///
/// 데이터 경로: ~/.openxgram/keystore/<name>.json
pub struct FsKeystore {
    base_dir: PathBuf,
}

impl FsKeystore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// 기본 경로 ~/.openxgram/keystore/ 사용
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home).join(".openxgram").join("keystore")
    }

    fn key_path(&self, name: &str) -> PathBuf {
        self.base_dir.join(format!("{name}.json"))
    }

    fn ensure_dir(&self) -> Result<(), KeystoreError> {
        std::fs::create_dir_all(&self.base_dir)?;
        Ok(())
    }

    /// 패스워드에서 ChaCha20-Poly1305 키 파생 (Argon2id)
    fn derive_encryption_key(password: &str, salt: &SaltString) -> Result<[u8; 32], KeystoreError> {
        let params = Params::new(
            65536, // m_cost: 64 MiB
            3,     // t_cost
            1,     // p_cost
            Some(32),
        )
        .map_err(|e| KeystoreError::Crypto(format!("argon2 params error: {e}")))?;

        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

        let hash = argon2
            .hash_password(password.as_bytes(), salt)
            .map_err(|e| KeystoreError::Crypto(format!("argon2 hash error: {e}")))?;

        let hash_bytes = hash
            .hash
            .ok_or_else(|| KeystoreError::Crypto("argon2 hash output missing".to_string()))?;

        let mut key = [0u8; 32];
        key.copy_from_slice(hash_bytes.as_bytes());
        Ok(key)
    }

    /// 비밀키 바이트를 암호화하여 저장
    fn encrypt_and_save(
        &self,
        name: &str,
        secret_bytes: &[u8],
        address: &AgentAddress,
        path_str: &str,
        password: &str,
    ) -> Result<(), KeystoreError> {
        self.ensure_dir()?;

        // salt 생성
        let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);

        // 암호화 키 파생
        let mut enc_key = Self::derive_encryption_key(password, &salt)?;

        // ChaCha20-Poly1305 암호화
        let cipher = ChaCha20Poly1305::new((&enc_key).into());
        let nonce_bytes: [u8; 12] = {
            use chacha20poly1305::aead::rand_core::RngCore;
            let mut b = [0u8; 12];
            AeadOsRng.fill_bytes(&mut b);
            b
        };
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, secret_bytes)
            .map_err(|e| KeystoreError::Crypto(format!("encrypt error: {e}")))?;

        enc_key.zeroize();

        // 인증 태그는 ciphertext 마지막 16바이트 (AEAD)
        let tag_start = ciphertext.len().saturating_sub(16);
        let mac = hex::encode(&ciphertext[tag_start..]);
        let ciphertext_hex = hex::encode(&ciphertext[..tag_start]);

        let now = chrono_now();

        let keyfile = EncryptedKeyFile {
            version: 3,
            name: name.to_string(),
            address: address.to_string(),
            derivation_path: path_str.to_string(),
            created_at: now,
            crypto: CryptoParams {
                cipher: "chacha20-poly1305".to_string(),
                ciphertext: ciphertext_hex,
                nonce: hex::encode(nonce_bytes),
                kdf: "argon2id".to_string(),
                kdf_params: KdfParams {
                    m_cost: 65536,
                    t_cost: 3,
                    p_cost: 1,
                    output_len: 32,
                },
                salt: salt.to_string(),
                mac,
            },
        };

        let json = serde_json::to_string_pretty(&keyfile)?;
        std::fs::write(self.key_path(name), json)?;
        Ok(())
    }

    /// 파일에서 복호화하여 비밀키 반환
    fn load_and_decrypt(&self, name: &str, password: &str) -> Result<Vec<u8>, KeystoreError> {
        let path = self.key_path(name);
        if !path.exists() {
            return Err(KeystoreError::NotFound(name.to_string()));
        }

        let json = std::fs::read_to_string(&path)?;
        let keyfile: EncryptedKeyFile = serde_json::from_str(&json)?;

        // salt 파싱
        let salt = SaltString::from_b64(&keyfile.crypto.salt)
            .map_err(|e| KeystoreError::Crypto(format!("salt parse error: {e}")))?;

        // 암호화 키 파생
        let mut enc_key = Self::derive_encryption_key(password, &salt)?;

        let cipher = ChaCha20Poly1305::new((&enc_key).into());
        enc_key.zeroize();

        let nonce_bytes = hex::decode(&keyfile.crypto.nonce)?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        // ciphertext + mac 재결합 (AEAD 복호화용)
        let mut ciphertext = hex::decode(&keyfile.crypto.ciphertext)?;
        let mac_bytes = hex::decode(&keyfile.crypto.mac)?;
        ciphertext.extend_from_slice(&mac_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| KeystoreError::InvalidPassword)?;

        Ok(plaintext)
    }
}

impl Keystore for FsKeystore {
    fn create(&self, name: &str, password: &str) -> Result<(AgentAddress, String), KeystoreError> {
        let path = self.key_path(name);
        if path.exists() {
            return Err(KeystoreError::Crypto(format!(
                "key '{name}' already exists"
            )));
        }

        let mnemonic = Mnemonic::generate(MnemonicLanguage::English);
        let seed = mnemonic.to_seed("");
        let deriv = DerivationPath::new(0, 0);
        let keypair = derive_keypair(&seed, &deriv)?;
        let address = keypair.address.clone();
        let path_str = deriv.to_bip44_string();

        let secret = keypair.secret_key_bytes();
        self.encrypt_and_save(name, &secret, &address, &path_str, password)?;

        let phrase = mnemonic.phrase().to_string();
        Ok((address, phrase))
    }

    fn import(
        &self,
        name: &str,
        phrase: &str,
        password: &str,
    ) -> Result<AgentAddress, KeystoreError> {
        let mnemonic = Mnemonic::from_phrase(phrase)?;
        let seed = mnemonic.to_seed("");
        let deriv = DerivationPath::new(0, 0);
        let keypair = derive_keypair(&seed, &deriv)?;
        let address = keypair.address.clone();
        let path_str = deriv.to_bip44_string();

        let secret = keypair.secret_key_bytes();
        self.encrypt_and_save(name, &secret, &address, &path_str, password)?;

        Ok(address)
    }

    fn load(&self, name: &str, password: &str) -> Result<Keypair, KeystoreError> {
        let secret = self.load_and_decrypt(name, password)?;
        Keypair::from_secret_bytes(&secret)
    }

    fn list(&self) -> Result<Vec<KeystoreEntry>, KeystoreError> {
        self.ensure_dir()?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let json = std::fs::read_to_string(&path)?;
            let kf: EncryptedKeyFile = serde_json::from_str(&json)?;
            entries.push(KeystoreEntry {
                name: kf.name,
                address: kf.address,
                derivation_path: kf.derivation_path,
                created_at: kf.created_at,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn delete(&self, name: &str) -> Result<(), KeystoreError> {
        let path = self.key_path(name);
        if !path.exists() {
            return Err(KeystoreError::NotFound(name.to_string()));
        }
        std::fs::remove_file(&path)?;
        Ok(())
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC3339 근사 — chrono 없이 KST+9 반영
    let kst_secs = secs + 9 * 3600;
    let days = kst_secs / 86400;
    let time_of_day = kst_secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    // 간단 날짜 계산 (1970-01-01 기준)
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}+09:00")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dim in &months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}
