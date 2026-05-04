//! `xgram identity` — W3C DID + 한국 OpenDID + OmniOne Open DID 호환.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Subcommand, ValueEnum};
use openxgram_did::{
    did_document, did_key_from_master, issue_vc, omnione_format, opendid_kr_format, verify_vc,
};
use openxgram_keystore::{FsKeystore, Keystore};

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum DidFormat {
    /// W3C did:key (default)
    Key,
    /// 한국디지털인증협회 OpenDID — did:opendid:{network}:{id}
    OpendidKr,
    /// OmniOne Open DID — did:omn:{id}
    Omnione,
}

#[derive(Subcommand, Debug)]
pub enum IdentityCli {
    /// DID 식별자 출력 — 기본 did:key, --format 으로 OpenDID/OmniOne
    Did {
        #[arg(long, default_value = "master")]
        name: String,
        #[arg(long)]
        password: String,
        #[arg(long, value_enum, default_value_t = DidFormat::Key)]
        format: DidFormat,
        #[arg(long, default_value = "mainnet")]
        network: String,
    },
    /// W3C DID Document JSON-LD 출력
    DidDocument {
        #[arg(long, default_value = "master")]
        name: String,
        #[arg(long)]
        password: String,
    },
    /// W3C Verifiable Credential 발급 (master 키로 ES256K 서명)
    IssueVc {
        #[arg(long, default_value = "master")]
        name: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        claims: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// VC 검증 — issuer pubkey (compressed sec1 hex 33B) 로 서명 확인
    VerifyVc {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        issuer_pubkey: String,
    },
}

pub fn run(ks: FsKeystore, action: IdentityCli) -> Result<()> {
    match action {
        IdentityCli::Did {
            name,
            password,
            format,
            network,
        } => {
            let kp = ks.load(&name, &password)?;
            let out = match format {
                DidFormat::Key => did_key_from_master(&kp)?,
                DidFormat::OpendidKr => opendid_kr_format(&kp, &network)?,
                DidFormat::Omnione => omnione_format(&kp)?,
            };
            println!("{out}");
        }
        IdentityCli::DidDocument { name, password } => {
            let kp = ks.load(&name, &password)?;
            let did = did_key_from_master(&kp)?;
            let doc = did_document(&did, &kp)?;
            println!("{}", serde_json::to_string_pretty(&doc)?);
        }
        IdentityCli::IssueVc {
            name,
            password,
            subject,
            claims,
            output,
        } => {
            let kp = ks.load(&name, &password)?;
            let issuer = did_key_from_master(&kp)?;
            let claims_v: serde_json::Value =
                serde_json::from_str(&claims).context("--claims 가 유효한 JSON 이 아닙니다")?;
            let vc = issue_vc(&issuer, &subject, claims_v, &kp)?;
            let pretty = serde_json::to_string_pretty(&vc)?;
            if let Some(path) = output {
                std::fs::write(&path, &pretty)
                    .with_context(|| format!("VC 파일 쓰기 실패: {}", path.display()))?;
                println!("VC 저장: {} (issuer={issuer})", path.display());
            } else {
                println!("{pretty}");
            }
        }
        IdentityCli::VerifyVc {
            input,
            issuer_pubkey,
        } => {
            let raw = std::fs::read_to_string(&input)
                .with_context(|| format!("VC 파일 읽기 실패: {}", input.display()))?;
            let vc: serde_json::Value = serde_json::from_str(&raw).context("VC JSON 파싱 실패")?;
            let pk = hex::decode(issuer_pubkey.trim())
                .map_err(|e| anyhow!("--issuer-pubkey hex 디코드 실패: {e}"))?;
            if pk.len() != 33 {
                return Err(anyhow!(
                    "issuer-pubkey 는 compressed sec1 33 bytes 여야 합니다 (got {})",
                    pk.len()
                ));
            }
            let ok = verify_vc(&vc, &pk)?;
            if ok {
                println!("VC 검증 통과");
            } else {
                return Err(anyhow!("VC 검증 실패"));
            }
        }
    }
    Ok(())
}
