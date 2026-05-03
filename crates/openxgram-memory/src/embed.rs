//! Embedder trait + DummyEmbedder (CI/test 결정성 384d) + FastEmbedder
//! (optional `fastembed` feature, multilingual-e5-small 384d 의미 임베딩).

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

/// multilingual-e5-small 기반 실 의미 임베딩.
///
/// 첫 호출 시 ONNX 모델 (~560MB) 다운로드. `fastembed` feature 활성 시 빌드.
#[cfg(feature = "fastembed")]
pub struct FastEmbedder {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
}

#[cfg(feature = "fastembed")]
impl FastEmbedder {
    pub fn new() -> Result<Self, anyhow::Error> {
        let model = fastembed::TextEmbedding::try_new(fastembed::InitOptions::new(
            fastembed::EmbeddingModel::MultilingualE5Small,
        ))?;
        Ok(Self {
            model: std::sync::Mutex::new(model),
        })
    }
}

#[cfg(feature = "fastembed")]
impl Embedder for FastEmbedder {
    fn dim(&self) -> usize {
        EMBED_DIM
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut model = self.model.lock().expect("FastEmbedder mutex poisoned");
        model
            .embed(vec![text], None)
            .expect("fastembed embed failed")
            .into_iter()
            .next()
            .expect("fastembed returned empty result")
    }
}

/// 런타임 임베더 선택. `fastembed` feature 가 빌드되어 있으면 FastEmbedder,
/// 그렇지 않거나 `XGRAM_EMBEDDER=dummy` 면 DummyEmbedder.
///
/// 첫 호출 시 ONNX 모델 (~560MB) 다운로드 — daemon 시작 시 1회 lazy.
#[allow(unused_variables)] // dummy fallback path doesn't use feature flags
pub fn default_embedder() -> anyhow::Result<Box<dyn Embedder + Send + Sync>> {
    let force_dummy = std::env::var("XGRAM_EMBEDDER").as_deref() == Ok("dummy");

    #[cfg(feature = "fastembed")]
    if !force_dummy {
        let fe = FastEmbedder::new()
            .map_err(|e| anyhow::anyhow!("fastembed init 실패: {e}"))?;
        return Ok(Box::new(fe));
    }

    Ok(Box::new(DummyEmbedder))
}
