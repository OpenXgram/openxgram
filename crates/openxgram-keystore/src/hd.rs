use coins_bip32::xkeys::XPriv;

use crate::error::KeystoreError;
use crate::keypair::Keypair;

/// BIP44 파생 경로 — m/44'/60'/account'/0/index
#[derive(Debug, Clone)]
pub struct DerivationPath {
    pub account: u32,
    pub index: u32,
}

impl DerivationPath {
    pub fn new(account: u32, index: u32) -> Self {
        Self { account, index }
    }

    /// BIP44 표준 경로 문자열 반환
    pub fn to_bip44_string(&self) -> String {
        format!("m/44'/60'/{}'/0/{}", self.account, self.index)
    }
}

impl std::fmt::Display for DerivationPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_bip44_string())
    }
}

/// BIP39 시드에서 HD 키페어 파생
///
/// path: m/44'/60'/account'/0/index
pub fn derive_keypair(seed: &[u8; 64], path: &DerivationPath) -> Result<Keypair, KeystoreError> {
    let path_str = path.to_bip44_string();

    // BIP32 master key from seed
    let master = XPriv::root_from_seed(seed.as_ref(), None)
        .map_err(|e| KeystoreError::Crypto(format!("master key derivation failed: {e}")))?;

    // BIP32 경로 파생 — derive_path는 문자열도 받음
    let child = master.derive_path(path_str.as_str()).map_err(|e| {
        KeystoreError::Crypto(format!("path derivation failed for {path_str}: {e}"))
    })?;

    // XPriv → SigningKey → 비밀키 바이트
    // AsRef<ecdsa::SigningKey> for XPriv 구현 활용
    let signing_key: &k256::ecdsa::SigningKey = child.as_ref();
    let secret_bytes = signing_key.to_bytes();

    Keypair::from_secret_bytes(&secret_bytes)
}
