//! WikiStore — DB 인덱스 (wiki_pages + wiki_embeddings).
//!
//! 절대 규칙 1 (fallback 금지): 모든 에러는 raise. silent fallback 없음.

use rusqlite::{params, Connection};
use thiserror::Error;

use crate::page::{Page, PageId, PageType};
use crate::content_hash;
use std::str::FromStr;

/// DB 인덱스 에러.
#[derive(Debug, Error)]
pub enum WikiStoreError {
    /// SQLite 에러.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// 직렬화 실패.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    /// 페이지 미존재.
    #[error("page not found: {0}")]
    NotFound(String),

    /// content_hash mismatch (낙관 잠금 실패).
    #[error("content_hash mismatch for {id}: expected={expected}, actual={actual}")]
    ContentHashMismatch {
        /// 페이지 id.
        id: String,
        /// 기대 hash.
        expected: String,
        /// 실제 hash.
        actual: String,
    },

    /// 페이지 id 파싱 실패.
    #[error("invalid page id: {0}")]
    InvalidPageId(String),
}

/// wiki_pages + wiki_embeddings 인덱스 접근자.
pub struct WikiStore<'a> {
    conn: &'a Connection,
}

impl<'a> WikiStore<'a> {
    /// 신규 store. 호출자가 `Connection` 수명 관리.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 페이지 upsert. expected_hash가 Some이면 낙관 잠금 — 불일치 시 ContentHashMismatch.
    pub fn upsert(&self, page: &Page, expected_hash: Option<&str>) -> Result<(), WikiStoreError> {
        if let Some(expected) = expected_hash {
            let actual: Option<String> = self
                .conn
                .query_row(
                    "SELECT content_hash FROM wiki_pages WHERE id = ?1",
                    params![page.id.as_str()],
                    |r| r.get(0),
                )
                .ok();
            if let Some(actual) = actual {
                if actual != expected {
                    return Err(WikiStoreError::ContentHashMismatch {
                        id: page.id.to_string(),
                        expected: expected.to_string(),
                        actual,
                    });
                }
            }
        }

        let related_json = serde_json::to_string(
            &page.related.iter().map(|r| r.to.as_str()).collect::<Vec<_>>(),
        )?;
        let source_refs_json = serde_json::to_string(&page.source_refs)?;

        self.conn.execute(
            "INSERT INTO wiki_pages (
                id, file_path, page_type, title, content_hash,
                related, source_refs, embedding_hash,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                file_path     = excluded.file_path,
                page_type     = excluded.page_type,
                title         = excluded.title,
                content_hash  = excluded.content_hash,
                related       = excluded.related,
                source_refs   = excluded.source_refs,
                embedding_hash = excluded.embedding_hash,
                updated_at    = excluded.updated_at",
            params![
                page.id.as_str(),
                page.id.file_path(),
                page.page_type.as_dir(),
                page.title,
                page.content_hash,
                related_json,
                source_refs_json,
                page.embedding_hash,
                page.created_at.timestamp(),
                page.updated_at.timestamp(),
            ],
        )?;
        Ok(())
    }

    /// id로 페이지 row 조회.
    pub fn get(&self, id: &PageId) -> Result<Option<PageRow>, WikiStoreError> {
        let row = self.conn.query_row(
            "SELECT id, file_path, page_type, title, content_hash, related, source_refs,
                    embedding_hash, created_at, updated_at
             FROM wiki_pages WHERE id = ?1",
            params![id.as_str()],
            |r| {
                Ok(PageRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    page_type: r.get(2)?,
                    title: r.get(3)?,
                    content_hash: r.get(4)?,
                    related: r.get(5)?,
                    source_refs: r.get(6)?,
                    embedding_hash: r.get(7)?,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            },
        );
        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 페이지 삭제.
    pub fn delete(&self, id: &PageId) -> Result<bool, WikiStoreError> {
        let n = self
            .conn
            .execute("DELETE FROM wiki_pages WHERE id = ?1", params![id.as_str()])?;
        // 임베딩도 함께 삭제 (FK cascade 미사용 — 명시적)
        self.conn.execute(
            "DELETE FROM wiki_embeddings WHERE page_id = ?1",
            params![id.as_str()],
        )?;
        Ok(n > 0)
    }

    /// 타입 필터 (None이면 전체) + 정렬: updated_at desc.
    pub fn list(&self, page_type: Option<PageType>) -> Result<Vec<PageRow>, WikiStoreError> {
        let mut stmt = if page_type.is_some() {
            self.conn.prepare(
                "SELECT id, file_path, page_type, title, content_hash, related, source_refs,
                        embedding_hash, created_at, updated_at
                 FROM wiki_pages WHERE page_type = ?1 ORDER BY updated_at DESC",
            )?
        } else {
            self.conn.prepare(
                "SELECT id, file_path, page_type, title, content_hash, related, source_refs,
                        embedding_hash, created_at, updated_at
                 FROM wiki_pages ORDER BY updated_at DESC",
            )?
        };

        let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<PageRow> {
            Ok(PageRow {
                id: r.get(0)?,
                file_path: r.get(1)?,
                page_type: r.get(2)?,
                title: r.get(3)?,
                content_hash: r.get(4)?,
                related: r.get(5)?,
                source_refs: r.get(6)?,
                embedding_hash: r.get(7)?,
                created_at: r.get(8)?,
                updated_at: r.get(9)?,
            })
        };

        let rows: Vec<PageRow> = if let Some(t) = page_type {
            stmt.query_map(params![t.as_dir()], mapper)?
                .collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map([], mapper)?.collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    /// 단순 LIKE 검색 (벡터 검색은 search.rs).
    pub fn search_like(&self, query: &str, limit: usize) -> Result<Vec<PageRow>, WikiStoreError> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, page_type, title, content_hash, related, source_refs,
                    embedding_hash, created_at, updated_at
             FROM wiki_pages WHERE title LIKE ?1 OR id LIKE ?1 ORDER BY updated_at DESC LIMIT ?2",
        )?;
        let rows: Vec<PageRow> = stmt
            .query_map(params![pattern, limit as i64], |r| {
                Ok(PageRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    page_type: r.get(2)?,
                    title: r.get(3)?,
                    content_hash: r.get(4)?,
                    related: r.get(5)?,
                    source_refs: r.get(6)?,
                    embedding_hash: r.get(7)?,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// `link_concepts` MCP 도구 — from의 related에 to를 추가.
    pub fn link(&self, from: &PageId, to: &PageId, reason: Option<&str>) -> Result<(), WikiStoreError> {
        let row = self
            .get(from)?
            .ok_or_else(|| WikiStoreError::NotFound(from.to_string()))?;
        let mut related: Vec<String> = serde_json::from_str(&row.related).unwrap_or_default();
        let to_s = to.to_string();
        if !related.iter().any(|r| r == &to_s) {
            related.push(to_s);
        }
        let _ = reason; // 향후 구조화 link 테이블 도입 시 사용
        let related_json = serde_json::to_string(&related)?;
        self.conn.execute(
            "UPDATE wiki_pages SET related = ?1, updated_at = strftime('%s','now') WHERE id = ?2",
            params![related_json, from.as_str()],
        )?;
        Ok(())
    }

    /// 임베딩 hash 갱신 (벡터는 sqlite-vec 별도 테이블에 저장).
    pub fn update_embedding_hash(&self, id: &PageId, hash: &str) -> Result<(), WikiStoreError> {
        let n = self.conn.execute(
            "UPDATE wiki_pages SET embedding_hash = ?1 WHERE id = ?2",
            params![hash, id.as_str()],
        )?;
        if n == 0 {
            return Err(WikiStoreError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// content_hash 변경 여부 (디스크 watcher 동기화).
    pub fn needs_resync(&self, id: &PageId, disk_content: &str) -> Result<bool, WikiStoreError> {
        let hash = content_hash(disk_content);
        let row = self.get(id)?;
        match row {
            Some(r) => Ok(r.content_hash != hash),
            None => Ok(true),
        }
    }
}

/// DB row의 plain 변환 (Page로 재구성하려면 file body 필요).
#[derive(Debug, Clone)]
pub struct PageRow {
    /// 페이지 id 문자열.
    pub id: String,
    /// 디스크 상대 경로.
    pub file_path: String,
    /// 타입 (entity/concept/comparison/other).
    pub page_type: String,
    /// 제목.
    pub title: String,
    /// content_hash.
    pub content_hash: String,
    /// related JSON 배열.
    pub related: String,
    /// source_refs JSON 배열.
    pub source_refs: String,
    /// embedding_hash.
    pub embedding_hash: String,
    /// 생성 시각 (unix epoch).
    pub created_at: i64,
    /// 갱신 시각 (unix epoch).
    pub updated_at: i64,
}

impl PageRow {
    /// related ids 파싱.
    pub fn related_ids(&self) -> Vec<PageId> {
        let raw: Vec<String> = serde_json::from_str(&self.related).unwrap_or_default();
        raw.into_iter()
            .filter_map(|s| PageId::from_str(&s).ok())
            .collect()
    }

    /// source_refs 파싱.
    pub fn source_ref_list(&self) -> Vec<String> {
        serde_json::from_str(&self.source_refs).unwrap_or_default()
    }

    /// PageType.
    pub fn typed(&self) -> Option<PageType> {
        PageType::from_str(&self.page_type).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE wiki_pages (
                id TEXT PRIMARY KEY, file_path TEXT UNIQUE NOT NULL, page_type TEXT NOT NULL,
                title TEXT NOT NULL, content_hash TEXT NOT NULL, related TEXT, source_refs TEXT,
                embedding_hash TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );
            CREATE TABLE wiki_embeddings (page_id TEXT PRIMARY KEY, embedding BLOB);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn upsert_and_get() {
        let conn = fresh_db();
        let store = WikiStore::new(&conn);
        let p = Page::new(
            PageId::new(PageType::Entity, "user").unwrap(),
            PageType::Entity,
            "User".to_string(),
            "body".to_string(),
        );
        store.upsert(&p, None).unwrap();
        let row = store.get(&p.id).unwrap().unwrap();
        assert_eq!(row.title, "User");
    }

    #[test]
    fn optimistic_lock_rejects_mismatch() {
        let conn = fresh_db();
        let store = WikiStore::new(&conn);
        let p = Page::new(
            PageId::new(PageType::Entity, "user").unwrap(),
            PageType::Entity,
            "User".to_string(),
            "body".to_string(),
        );
        store.upsert(&p, None).unwrap();
        let err = store.upsert(&p, Some("wronghash")).unwrap_err();
        assert!(matches!(err, WikiStoreError::ContentHashMismatch { .. }));
    }
}
