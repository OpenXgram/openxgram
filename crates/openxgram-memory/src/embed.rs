//! Embedder trait + DummyEmbedder (CI/test 결정성 384d).
//!
//! 실 의미 임베딩(multilingual-e5-small fastembed)은 후속 PR.

use sha2::{Digest, Sha256};

pub const EMBED_DIM: usize = 384;

/// 임베딩 추상화. 어떤 구현체도 같은 차원·동일 dtype.
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// SHA256 해시 기반 결정성 384d 임베딩. 같은 텍스트 → 같은 벡터.
/// 의미 유사도는 보장하지 않으나, 회상 알고리즘 검증에 충분.
pub struct DummyEmbedder;

impl Embedder for DummyEmbedder {
    fn dim(&self) -> usize {
        EMBED_DIM
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();

        let mut out = Vec::with_capacity(EMBED_DIM);
        for i in 0..EMBED_DIM {
            let byte = hash[i % 32];
            let mixed = (byte as f32 - 128.0) / 128.0
                + (i as f32 / EMBED_DIM as f32 - 0.5) * 0.001;
            out.push(mixed);
        }
        // L2 정규화 — distance 비교 일관성
        let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut out {
                *v /= norm;
            }
        }
        out
    }
}
