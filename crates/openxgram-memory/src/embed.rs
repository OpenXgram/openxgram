//! Embedder trait + DummyEmbedder (CI/test 결정성 384d) + FastEmbedder
//! (optional `fastembed` feature, multilingual-e5-small 384d 의미 임베딩).

use sha2::{Digest, Sha256};

pub const EMBED_DIM: usize = 384;

/// 임베딩 추상화. 어떤 구현체도 같은 차원·동일 dtype.
///
/// multilingual-e5 계열은 문서엔 `passage: `, 쿼리엔 `query: ` prefix 필수.
/// `embed_passage` / `embed_query` 를 분리해 호출 측이 명시적으로 선택한다.
/// 기본 구현(`embed`)은 하위 호환용 — 새 코드는 반드시 passage/query 변형 사용.
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;

    /// prefix 없는 raw embed — 하위 호환. 새 코드는 embed_passage/embed_query 사용.
    fn embed(&self, text: &str) -> Vec<f32>;

    /// 문서 저장용 임베딩 (`passage: <text>` prefix).
    fn embed_passage(&self, text: &str) -> Vec<f32> {
        self.embed(text)
    }

    /// 쿼리(검색어)용 임베딩 (`query: <text>` prefix).
    fn embed_query(&self, text: &str) -> Vec<f32> {
        self.embed(text)
    }
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
            let mixed = (byte as f32 - 128.0) / 128.0 + (i as f32 / EMBED_DIM as f32 - 0.5) * 0.001;
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
impl FastEmbedder {
    fn embed_with_prefix(&self, prefix: &str, text: &str) -> Vec<f32> {
        let prefixed = format!("{}{}", prefix, text);
        let mut model = self.model.lock().expect("FastEmbedder mutex poisoned");
        model
            .embed(vec![prefixed.as_str()], None)
            .expect("fastembed embed failed")
            .into_iter()
            .next()
            .expect("fastembed returned empty result")
    }
}

#[cfg(feature = "fastembed")]
impl Embedder for FastEmbedder {
    fn dim(&self) -> usize {
        EMBED_DIM
    }

    /// 하위 호환 — passage prefix 적용 (저장 기본값).
    fn embed(&self, text: &str) -> Vec<f32> {
        self.embed_with_prefix("passage: ", text)
    }

    fn embed_passage(&self, text: &str) -> Vec<f32> {
        self.embed_with_prefix("passage: ", text)
    }

    fn embed_query(&self, text: &str) -> Vec<f32> {
        self.embed_with_prefix("query: ", text)
    }
}

/// 현재 빌드/환경에서 default_embedder() 가 반환할 모드의 라벨.
/// "fastembed" / "fastembed-overridden-dummy" / "dummy".
pub fn embedder_mode_label() -> &'static str {
    let force_dummy = std::env::var("XGRAM_EMBEDDER").as_deref() == Ok("dummy");
    #[cfg(feature = "fastembed")]
    {
        if force_dummy {
            return "fastembed-overridden-dummy";
        }
        return "fastembed";
    }
    #[cfg(not(feature = "fastembed"))]
    {
        let _ = force_dummy;
        "dummy"
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
        let fe = FastEmbedder::new().map_err(|e| anyhow::anyhow!("fastembed init 실패: {e}"))?;
        return Ok(Box::new(fe));
    }

    Ok(Box::new(DummyEmbedder))
}
