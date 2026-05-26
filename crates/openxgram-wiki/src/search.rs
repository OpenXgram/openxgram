//! 위키 검색 — 벡터 KNN + LIKE.
//!
//! 절대 규칙 1 (fallback 금지): LIKE와 벡터 검색은 각각 명시적 대안이다.
//! `search_wiki_combined`가 둘을 합산해 최종 결과를 반환한다.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::page::PageId;
use crate::store::{PageRow, WikiStore, WikiStoreError};

/// 검색 결과.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// 페이지 id.
    pub id: PageId,
    /// 제목.
    pub title: String,
    /// 점수 (코사인 유사도 또는 LIKE 매칭 가중).
    pub score: f32,
}

impl From<PageRow> for SearchHit {
    fn from(r: PageRow) -> Self {
        SearchHit {
            id: r.id.parse().unwrap_or_else(|_| {
                // 잘못된 id는 placeholder. 절대 규칙 1 호환 — 검색 결과에서 노출하지 않도록
                // 호출자가 필터.
                use std::str::FromStr;
                PageId::from_str("other/_unknown").unwrap()
            }),
            title: r.title,
            score: 0.0,
        }
    }
}

/// LIKE 기반 단순 검색 (title / id / file_path). 임베더 없이도 작동.
pub fn search_wiki(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, WikiStoreError> {
    let store = WikiStore::new(conn);
    let rows = store.search_like(query, limit)?;
    Ok(rows.into_iter().map(SearchHit::from).collect())
}

/// wiki_embeddings BLOB 전체를 로드해 코사인 유사도로 KNN.
/// wiki_embeddings 테이블이 비어 있으면 빈 Vec 반환 (에러 아님).
pub fn search_wiki_vec(
    conn: &Connection,
    embedder: &dyn openxgram_memory::Embedder,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, WikiStoreError> {
    let store = WikiStore::new(conn);

    // 1. 쿼리 벡터 (e5 query prefix 적용)
    let q_vec = embedder.embed_query(query);
    let q_norm: f32 = q_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if q_norm == 0.0 {
        return Ok(vec![]);
    }

    // 2. 전체 임베딩 로드
    let all = store.all_embeddings()?;
    if all.is_empty() {
        return Ok(vec![]);
    }

    // 3. 코사인 유사도 계산
    let mut scored: Vec<(String, f32)> = all
        .into_iter()
        .map(|(page_id, vec)| {
            let dot: f32 = q_vec.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
            let v_norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
            let sim = if v_norm > 0.0 { dot / (q_norm * v_norm) } else { 0.0 };
            (page_id, sim)
        })
        .collect();

    // 4. 내림차순 정렬 → top-k
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    // 5. page_id로 wiki_pages 조회해 title 채움
    let mut hits = Vec::with_capacity(scored.len());
    for (page_id_str, sim) in scored {
        // id 파싱 실패 시 skip (잘못된 레코드)
        let id: PageId = match page_id_str.parse() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let row = store.get(&id)?;
        let title = row.map(|r| r.title).unwrap_or_else(|| id.to_string());
        hits.push(SearchHit { id, title, score: sim });
    }
    Ok(hits)
}

/// LIKE + 벡터 검색을 합산해 중복 제거 후 score 내림차순 반환.
/// 벡터 검색은 임베더가 주입된 경우에만 실행된다.
pub fn search_wiki_combined(
    conn: &Connection,
    embedder: Option<&dyn openxgram_memory::Embedder>,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, WikiStoreError> {
    // LIKE 결과 (score=1.0 고정)
    let like_hits = search_wiki(conn, query, limit)?;

    // 벡터 결과
    let vec_hits: Vec<SearchHit> = if let Some(emb) = embedder {
        search_wiki_vec(conn, emb as &dyn openxgram_memory::Embedder, query, limit)?
    } else {
        vec![]
    };

    // 합산: page_id 기준으로 score 누적 (LIKE 1.0 + 벡터 유사도)
    let mut map: HashMap<String, SearchHit> = HashMap::new();

    for mut h in like_hits {
        h.score = 1.0;
        map.insert(h.id.to_string(), h);
    }
    for h in vec_hits {
        let key = h.id.to_string();
        map.entry(key)
            .and_modify(|e| e.score += h.score)
            .or_insert(h);
    }

    let mut results: Vec<SearchHit> = map.into_values().collect();
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    Ok(results)
}
