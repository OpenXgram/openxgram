//! default_embedder() — feature gating + XGRAM_EMBEDDER 환경 검증.
//!
//! fastembed feature 활성/비활성 양쪽 모두 동일 진입점 사용 가능 검증.
//! 실제 ONNX 모델 다운로드는 검증 안 함 (CI 시간/대역폭 보호).

use openxgram_memory::{default_embedder, EMBED_DIM};

#[test]
fn returns_embedder_with_correct_dim() {
    // dummy 강제 — fastembed feature 가 켜져있어도 ONNX 다운로드 회피
    unsafe {
        std::env::set_var("XGRAM_EMBEDDER", "dummy");
    }
    let emb = default_embedder().expect("default_embedder failed");
    assert_eq!(emb.dim(), EMBED_DIM);
    let v = emb.embed("hello world");
    assert_eq!(v.len(), EMBED_DIM);
}

#[test]
fn deterministic_for_same_input_when_dummy() {
    unsafe {
        std::env::set_var("XGRAM_EMBEDDER", "dummy");
    }
    let emb = default_embedder().unwrap();
    let a = emb.embed("OpenXgram");
    let b = emb.embed("OpenXgram");
    assert_eq!(a, b);
    let c = emb.embed("different text");
    assert_ne!(a, c);
}
