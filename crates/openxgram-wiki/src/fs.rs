//! 디스크 I/O — `{XGRAM_DATA_DIR}/wiki/` 트리.
//!
//! 정본은 디스크. DB는 인덱스.

use std::path::{Path, PathBuf};
use tokio::fs;

use crate::page::{Page, PageId, PageType};
use crate::{WikiError, content_hash};

/// 위키 디스크 레이아웃 핸들러.
pub struct WikiFs {
    root: PathBuf,
}

impl WikiFs {
    /// `{XGRAM_DATA_DIR}/wiki/` 경로로 초기화.
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    /// 디렉토리 보장 (entity/, concept/, comparison/, other/).
    pub async fn ensure_dirs(&self) -> Result<(), WikiError> {
        fs::create_dir_all(&self.root).await?;
        for ty in [PageType::Entity, PageType::Concept, PageType::Comparison, PageType::Other] {
            fs::create_dir_all(self.root.join(ty.as_dir())).await?;
        }
        Ok(())
    }

    /// 페이지의 절대 경로.
    pub fn path(&self, id: &PageId) -> PathBuf {
        self.root.join(id.file_path())
    }

    /// 페이지 읽기 — 없으면 Ok(None).
    pub async fn read(&self, id: &PageId) -> Result<Option<Page>, WikiError> {
        let path = self.path(id);
        match fs::read_to_string(&path).await {
            Ok(src) => Ok(Some(Page::from_markdown(&src)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 페이지 쓰기. expected_hash가 Some이면 디스크의 현재 content_hash와 비교.
    pub async fn write(&self, page: &Page, expected_hash: Option<&str>) -> Result<(), WikiError> {
        if let Some(expected) = expected_hash {
            if let Some(existing_src) = self.read_raw(&page.id).await? {
                let actual = extract_body_hash(&existing_src);
                if actual.as_deref() != Some(expected) {
                    return Err(WikiError::ContentHashMismatch {
                        id: page.id.to_string(),
                        expected: expected.to_string(),
                        actual: actual.unwrap_or_else(|| "<none>".to_string()),
                    });
                }
            }
        }

        let path = self.path(page.id());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let md = page.to_markdown()?;

        // atomic write: tmp → rename
        let tmp = path.with_extension("md.tmp");
        fs::write(&tmp, &md).await?;
        fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// 페이지 삭제.
    pub async fn delete(&self, id: &PageId) -> Result<bool, WikiError> {
        let path = self.path(id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// 원본 markdown 그대로 읽기 (frontmatter 파싱 안 함).
    pub async fn read_raw(&self, id: &PageId) -> Result<Option<String>, WikiError> {
        let path = self.path(id);
        match fs::read_to_string(&path).await {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 디렉토리 전체 스캔 (sync 시 사용).
    pub async fn walk(&self) -> Result<Vec<(PageId, PathBuf)>, WikiError> {
        let mut out = Vec::new();
        for ty in [PageType::Entity, PageType::Concept, PageType::Comparison, PageType::Other] {
            let dir = self.root.join(ty.as_dir());
            if !dir.exists() {
                continue;
            }
            let mut rd = fs::read_dir(&dir).await?;
            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(id) = PageId::new(ty, stem) {
                            out.push((id, path));
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// 루트 경로.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Page {
    /// 페이지 id 참조.
    pub fn id(&self) -> &PageId {
        &self.id
    }
}

/// markdown 원본에서 body 영역의 hash 추출 (frontmatter 제외).
/// frontmatter가 없으면 전체 hash.
fn extract_body_hash(source: &str) -> Option<String> {
    let body = if source.starts_with("---") {
        let mut iter = source.splitn(3, "---");
        iter.next()?; // ""
        iter.next()?; // frontmatter
        iter.next()?.trim_start_matches('\n').to_string()
    } else {
        source.to_string()
    };
    Some(content_hash(body.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn ensure_dirs_creates_layout() {
        let tmp = tempdir().unwrap();
        let wf = WikiFs::new(tmp.path().join("wiki"));
        wf.ensure_dirs().await.unwrap();
        assert!(tmp.path().join("wiki/entity").exists());
        assert!(tmp.path().join("wiki/concept").exists());
        assert!(tmp.path().join("wiki/comparison").exists());
        assert!(tmp.path().join("wiki/other").exists());
    }

    #[tokio::test]
    async fn write_read_roundtrip() {
        let tmp = tempdir().unwrap();
        let wf = WikiFs::new(tmp.path().join("wiki"));
        wf.ensure_dirs().await.unwrap();

        let p = Page::new(
            PageId::new(PageType::Entity, "test").unwrap(),
            PageType::Entity,
            "Test".to_string(),
            "body".to_string(),
        );
        wf.write(&p, None).await.unwrap();

        let read = wf.read(&p.id).await.unwrap().unwrap();
        assert_eq!(read.id, p.id);
    }
}
