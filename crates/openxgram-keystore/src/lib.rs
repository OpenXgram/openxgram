//! openxgram-keystore — 키페어 관리, BIP39 시드, HD 파생, 서명·검증
//!
//! secp256k1 (Base 체인 호환) + BIP39 24단어 + BIP44 HD 파생 (m/44'/60'/N'/0/M)
//! 암호화: ChaCha20-Poly1305 + Argon2id (KDF)
//! 데이터: ~/.openxgram/keystore/

mod blob;
mod error;
mod hd;
mod keypair;
mod mnemonic;
mod storage;

pub use blob::{decrypt_blob, encrypt_blob};
pub use error::KeystoreError;
pub use hd::{derive_keypair, DerivationPath};
pub use keypair::{verify_with_pubkey, AgentAddress, Keypair};
pub use mnemonic::{Mnemonic, MnemonicLanguage};
pub use storage::{FsKeystore, Keystore, KeystoreEntry};
