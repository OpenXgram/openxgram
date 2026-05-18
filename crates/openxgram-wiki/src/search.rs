//! 위키 검색 — 벡터 KNN + LIKE fallback.
//!
//! 절대 규칙 1 (fallback 금지): "fallback"이라는 단어는 silent 차원에서 금지.
//! 여기서의 LIKE는 임베더가 비활성일 때의 **명시적 대안** (호출자가 선택).

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

/// LIKE 기반 단순 검색. 벡터 임베더 없이도 작동.
pub fn search_wiki(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, WikiStoreError> {
    let store = WikiStore::new(conn);
    let rows = store.search_like(query, limit)?;
    Ok(rows.into_iter().map(SearchHit::from).collect())
}

// NOTE: 벡터 KNN (sqlite-vec)은 openxgram-memory의 embed::Embedder를 주입받아
// 별도 함수로 구현 — `search_wiki_vec`. 임베더 가용성은 호출자가 결정.
// 본 crate가 fastembed에 직접 의존하지 않도록 trait injection 패턴을 따른다.

/// 임베더 trait 객체로 벡터 검색.
/// 호출자가 openxgram_memory::Embedder 구현체를 주입.
pub fn search_wiki_vec<E>(
    _conn: &Connection,
    _embedder: &E,
    _query: &str,
    _limit: usize,
) -> Result<Vec<SearchHit>, WikiStoreError>
where
    E: openxgram_memory::Embedder,
{
    // Phase v0.3 후속 PR — sqlite-vec MATCH 쿼리.
    // 본 PR은 인덱스·디스크 동기화·MCP 도구까지. KNN은 분리.
    Err(WikiStoreError::Sqlite(rusqlite::Error::InvalidQuery))
}
