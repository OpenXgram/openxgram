//! MCP 도구 핸들러 — 5개 신규 도구 (PRD-OpenXgram §4.1).
//!
//! - `read_wiki_page(topic)`               → 페이지 본문 + frontmatter
//! - `write_wiki_page(topic, content, type?)` → 생성/업데이트
//! - `link_concepts(from, to, reason?)`    → 크로스링크
//! - `search_wiki(query, k?=5)`            → LIKE 검색 (벡터 검색은 별도)
//! - `list_wiki(type?)`                    → 페이지 목록
//!
//! 본 모듈은 도메인 핸들러만 제공. JSON-RPC 어댑터는 openxgram-mcp가 래핑.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::fs::WikiFs;
use crate::page::{Page, PageId, PageType};
use crate::search::{search_wiki_combined, SearchHit};
use crate::store::{WikiStore, WikiStoreError};
use crate::WikiError;

/// 5개 MCP 도구의 도메인 핸들러.
pub struct WikiTools<'a> {
    fs: &'a WikiFs,
    conn: &'a Connection,
}

impl<'a> WikiTools<'a> {
    /// 신규 핸들러.
    pub fn new(fs: &'a WikiFs, conn: &'a Connection) -> Self {
        Self { fs, conn }
    }

    /// `read_wiki_page` — 페이지 본문 반환.
    ///
    /// topic은 `{type}/{slug}` 형식(정규) 또는 raw DB id (legacy) 모두 허용.
    /// raw id의 경우 DB에서 file_path를 조회해 PageId를 재구성한다.
    pub async fn read(&self, topic: &str) -> Result<ReadResult, WikiError> {
        // 1. 정규 형식 시도
        let id = match PageId::from_str(topic) {
            Ok(id) => id,
            Err(_) => {
                // 2. raw id로 DB 조회 → file_path에서 PageId 재구성
                let store = WikiStore::new(self.conn);
                let row = store
                    .get_by_raw_id(topic)?
                    .ok_or_else(|| WikiError::InvalidPageId(topic.to_string()))?;
                // file_path 형식: "wiki/{type}/{slug}.md" 또는 "{type}/{slug}.md"
                let fp = row.file_path.trim_start_matches("wiki/");
                let fp = fp.trim_end_matches(".md");
                PageId::from_str(fp)
                    .map_err(|_| WikiError::InvalidPageId(topic.to_string()))?
            }
        };
        // 1차: id 기반 정규 경로 시도
        let page_opt = self.fs.read(&id).await?;

        // 2차: DB에 등록된 file_path로 직접 읽기 (legacy "wiki/" prefix 등 경로 불일치 대응)
        let page_opt = if page_opt.is_none() {
            let store = WikiStore::new(self.conn);
            if let Some(row) = store.get(&id)? {
                // file_path는 "wiki/concept/foo.md" 또는 "concept/foo.md" 형태
                let rel = row.file_path.trim_start_matches("wiki/");
                let rel = rel.trim_end_matches(".md");
                if let Ok(alt_id) = rel.parse::<PageId>() {
                    self.fs.read(&alt_id).await?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            page_opt
        };

        match page_opt {
            Some(page) => Ok(ReadResult {
                id: page.id.to_string(),
                page_type: page.page_type.to_string(),
                title: page.title.clone(),
                body: page.body.clone(),
                related: page.related.iter().map(|r| r.to.to_string()).collect(),
                source_refs: page.source_refs.clone(),
                content_hash: page.content_hash.clone(),
            }),
            None => Err(WikiError::Other(format!("page not found: {}", topic))),
        }
    }

    /// `write_wiki_page` — 신규 생성 또는 업데이트 (낙관 잠금).
    pub async fn write(
        &self,
        topic: &str,
        content: &str,
        page_type: Option<&str>,
        expected_hash: Option<&str>,
    ) -> Result<WriteResult, WikiError> {
        let (parsed_type, slug) = PageId::parse(topic)?;
        let final_type = match page_type {
            Some(t) => PageType::from_str(t)?,
            None => parsed_type,
        };
        let id = PageId::new(final_type, &slug)?;

        let store = WikiStore::new(self.conn);
        let title = extract_first_h1(content).unwrap_or_else(|| slug.clone());

        // 기존 row의 created_at 보존
        let existing = store.get(&id)?;
        let mut page = Page::new(
            id.clone(),
            final_type,
            title,
            content.trim_end().to_string(),
        );
        if let Some(row) = &existing {
            page.created_at =
                chrono::DateTime::from_timestamp(row.created_at, 0).unwrap_or(page.created_at);
        }

        // 디스크 먼저 → DB 인덱스
        self.fs.write(&page, expected_hash).await?;
        store.upsert(&page, expected_hash)?;

        Ok(WriteResult {
            id: id.to_string(),
            content_hash: page.content_hash.clone(),
            created: existing.is_none(),
        })
    }

    /// `link_concepts` — from의 related에 to 추가.
    pub async fn link(
        &self,
        from: &str,
        to: &str,
        reason: Option<&str>,
    ) -> Result<LinkResult, WikiError> {
        let from_id = PageId::from_str(from)?;
        let to_id = PageId::from_str(to)?;
        let store = WikiStore::new(self.conn);
        store.link(&from_id, &to_id, reason)?;

        // 디스크의 frontmatter도 갱신 (정본 디스크 원칙)
        if let Some(mut page) = self.fs.read(&from_id).await? {
            if !page.related.iter().any(|r| r.to == to_id) {
                page.related.push(crate::page::Related {
                    to: to_id.clone(),
                    reason: reason.map(|s| s.to_string()),
                });
                page.updated_at = chrono::Utc::now();
                self.fs.write(&page, Some(&page.content_hash)).await?;
            }
        }

        Ok(LinkResult {
            from: from.to_string(),
            to: to.to_string(),
        })
    }

    /// `search_wiki` — LIKE + 벡터 검색 결합 (k 기본 5).
    /// embedder를 주입하면 wiki_embeddings KNN도 함께 실행된다.
    pub fn search(
        &self,
        query: &str,
        k: Option<usize>,
        embedder: Option<&dyn openxgram_memory::Embedder>,
    ) -> Result<Vec<SearchHit>, WikiError> {
        let limit = k.unwrap_or(5);
        Ok(search_wiki_combined(self.conn, embedder, query, limit)?)
    }

    /// `list_wiki` — 타입 필터 옵션.
    pub fn list(&self, page_type: Option<&str>) -> Result<Vec<ListEntry>, WikiError> {
        let ty = match page_type {
            Some(t) => Some(PageType::from_str(t)?),
            None => None,
        };
        let store = WikiStore::new(self.conn);
        let rows = store.list(ty)?;
        Ok(rows
            .into_iter()
            .map(|r| ListEntry {
                id: r.id.clone(),
                page_type: r.page_type.clone(),
                title: r.title.clone(),
                updated_at: r.updated_at,
            })
            .collect())
    }
}

/// `read_wiki_page` 결과.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadResult {
    /// 페이지 id.
    pub id: String,
    /// 타입.
    pub page_type: String,
    /// 제목.
    pub title: String,
    /// 본문.
    pub body: String,
    /// 관련 페이지 id.
    pub related: Vec<String>,
    /// 원천 ref.
    pub source_refs: Vec<String>,
    /// content_hash (낙관 잠금 키).
    pub content_hash: String,
}

/// `write_wiki_page` 결과.
#[derive(Debug, Serialize, Deserialize)]
pub struct WriteResult {
    /// 페이지 id.
    pub id: String,
    /// 새 content_hash.
    pub content_hash: String,
    /// 신규 생성 여부 (false면 업데이트).
    pub created: bool,
}

/// `link_concepts` 결과.
#[derive(Debug, Serialize, Deserialize)]
pub struct LinkResult {
    /// 출발 id.
    pub from: String,
    /// 도착 id.
    pub to: String,
}

/// `list_wiki` 한 entry.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListEntry {
    /// 페이지 id.
    pub id: String,
    /// 타입.
    pub page_type: String,
    /// 제목.
    pub title: String,
    /// 마지막 갱신 (unix epoch).
    pub updated_at: i64,
}

/// 본문 첫 `# Heading` 추출.
fn extract_first_h1(content: &str) -> Option<String> {
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// 변환: WikiStoreError → WikiError.
impl From<WikiStoreError> for WikiError {
    fn from(e: WikiStoreError) -> Self {
        match e {
            WikiStoreError::Sqlite(s) => WikiError::Other(format!("sqlite: {s}")),
            WikiStoreError::Serde(s) => WikiError::Serde(s),
            WikiStoreError::NotFound(id) => WikiError::Other(format!("not found: {id}")),
            WikiStoreError::ContentHashMismatch {
                id,
                expected,
                actual,
            } => WikiError::ContentHashMismatch {
                id,
                expected,
                actual,
            },
            WikiStoreError::InvalidPageId(id) => WikiError::InvalidPageId(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    async fn setup() -> (tempfile::TempDir, WikiFs, Connection) {
        let tmp = tempdir().unwrap();
        let wf = WikiFs::new(tmp.path().join("wiki"));
        wf.ensure_dirs().await.unwrap();
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
        (tmp, wf, conn)
    }

    #[tokio::test]
    async fn write_then_read() {
        let (_tmp, wf, conn) = setup().await;
        let tools = WikiTools::new(&wf, &conn);
        let w = tools
            .write("entity/alice", "# Alice\n\nBio body.", None, None)
            .await
            .unwrap();
        assert!(w.created);

        let r = tools.read("entity/alice").await.unwrap();
        assert_eq!(r.title, "Alice");
        assert!(r.body.contains("Bio body"));
    }

    #[tokio::test]
    async fn link_then_list() {
        let (_tmp, wf, conn) = setup().await;
        let tools = WikiTools::new(&wf, &conn);
        tools
            .write("entity/a", "# A\n\n", None, None)
            .await
            .unwrap();
        tools
            .write("entity/b", "# B\n\n", None, None)
            .await
            .unwrap();
        tools
            .link("entity/a", "entity/b", Some("관련"))
            .await
            .unwrap();

        let entries = tools.list(Some("entity")).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
