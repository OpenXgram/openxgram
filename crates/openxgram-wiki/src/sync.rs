//! 양방향 동기화: 디스크 ↔ DB 인덱스.
//!
//! 정책:
//!   - 디스크가 정본 (사용자 직접 수정 우선 — last-write-wins)
//!   - DB → 파일: `write_wiki_page` MCP 도구가 직접 호출
//!   - 파일 → DB: `Syncer::sync_disk_to_db()` (xgram wiki sync 명령 또는 notify watcher)
//!
//! 절대 규칙 1: silent fallback 금지. 충돌은 ContentHashMismatch 에러로 노출.

use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::fs::WikiFs;
use crate::page::PageId;
use crate::store::WikiStore;
use crate::{content_hash, WikiError};

/// 동기화 1회 실행 보고.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// 새로 인덱스에 추가된 페이지 수.
    pub added: usize,
    /// content_hash 변경으로 갱신된 페이지 수.
    pub updated: usize,
    /// 변경 없이 건너뛴 페이지 수.
    pub unchanged: usize,
    /// DB에는 있는데 디스크에서 사라진 페이지 (삭제 인덱스).
    pub removed: usize,
    /// 처리 중 발생한 에러 (id, 메시지).
    pub errors: Vec<(String, String)>,
}

/// 디스크 ↔ DB 동기화기.
pub struct Syncer<'a> {
    fs: &'a WikiFs,
    conn: &'a Connection,
}

impl<'a> Syncer<'a> {
    /// 신규 syncer.
    pub fn new(fs: &'a WikiFs, conn: &'a Connection) -> Self {
        Self { fs, conn }
    }

    /// 전체 디스크 스캔 → DB 갱신.
    /// notify watcher가 부재하거나 부팅 시 1회 호출.
    pub async fn sync_disk_to_db(&self) -> Result<SyncReport, WikiError> {
        let mut report = SyncReport::default();
        let store = WikiStore::new(self.conn);

        let on_disk = self.fs.walk().await?;
        let mut disk_ids: Vec<String> = Vec::with_capacity(on_disk.len());

        for (id, path) in on_disk {
            disk_ids.push(id.to_string());
            match self.sync_one(&store, &id, &path).await {
                Ok(SyncOutcome::Added) => report.added += 1,
                Ok(SyncOutcome::Updated) => report.updated += 1,
                Ok(SyncOutcome::Unchanged) => report.unchanged += 1,
                Err(e) => {
                    warn!(id = %id, error = %e, "sync_one failed");
                    report.errors.push((id.to_string(), e.to_string()));
                }
            }
        }

        // DB에 있지만 디스크에 없는 id → 삭제 (사용자가 파일을 지웠다는 의미)
        let db_rows = store.list(None)?;
        for row in db_rows {
            if !disk_ids.contains(&row.id) {
                if let Ok(id) = row.id.parse::<PageId>() {
                    if store.delete(&id)? {
                        report.removed += 1;
                        info!(id = %id, "removed from index (file deleted)");
                    }
                }
            }
        }

        Ok(report)
    }

    async fn sync_one(
        &self,
        store: &WikiStore<'_>,
        id: &PageId,
        _path: &Path,
    ) -> Result<SyncOutcome, WikiError> {
        // 디스크 정본 파싱
        let page = self.fs.read(id).await?.ok_or_else(|| {
            WikiError::Other(format!("walk reported {id} but read returned None"))
        })?;

        // 기존 row와 비교
        match store.get(id)? {
            None => {
                store.upsert(&page, None)?;
                debug!(id = %id, "added");
                Ok(SyncOutcome::Added)
            }
            Some(row) => {
                let disk_hash = content_hash(&page.body);
                if row.content_hash == disk_hash {
                    Ok(SyncOutcome::Unchanged)
                } else {
                    // 디스크가 더 최신 — DB 갱신. 낙관 잠금 미사용 (디스크 정본 원칙).
                    store.upsert(&page, None)?;
                    debug!(id = %id, "updated");
                    Ok(SyncOutcome::Updated)
                }
            }
        }
    }
}

enum SyncOutcome {
    Added,
    Updated,
    Unchanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{Page, PageId, PageType};
    use rusqlite::Connection;
    use tempfile::tempdir;

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

    #[tokio::test]
    async fn sync_adds_new_disk_pages() {
        let tmp = tempdir().unwrap();
        let wf = WikiFs::new(tmp.path().join("wiki"));
        wf.ensure_dirs().await.unwrap();

        let p = Page::new(
            PageId::new(PageType::Entity, "alpha").unwrap(),
            PageType::Entity,
            "Alpha".to_string(),
            "body".to_string(),
        );
        wf.write(&p, None).await.unwrap();

        let conn = fresh_db();
        let syncer = Syncer::new(&wf, &conn);
        let report = syncer.sync_disk_to_db().await.unwrap();
        assert_eq!(report.added, 1);
        assert_eq!(report.unchanged, 0);
        assert_eq!(report.errors.len(), 0);
    }

    #[tokio::test]
    async fn sync_removes_deleted_disk_pages() {
        let tmp = tempdir().unwrap();
        let wf = WikiFs::new(tmp.path().join("wiki"));
        wf.ensure_dirs().await.unwrap();

        let conn = fresh_db();

        // 1차: 파일 작성 + sync → DB에 row 1
        let p = Page::new(
            PageId::new(PageType::Entity, "alpha").unwrap(),
            PageType::Entity,
            "Alpha".to_string(),
            "body".to_string(),
        );
        wf.write(&p, None).await.unwrap();
        Syncer::new(&wf, &conn).sync_disk_to_db().await.unwrap();

        // 2차: 파일 삭제 + sync → DB row도 삭제
        wf.delete(&p.id).await.unwrap();
        let report = Syncer::new(&wf, &conn).sync_disk_to_db().await.unwrap();
        assert_eq!(report.removed, 1);
    }
}
