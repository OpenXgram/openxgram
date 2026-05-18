//! Wiki page 도메인 + frontmatter 파싱.
//!
//! 페이지 id 형식: `{type}/{slug}` (예: `entity/사용자`, `concept/마케팅-방법론`).
//! type은 PageType의 variant 중 하나여야 한다.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::{WikiError, content_hash};

/// 페이지 종류 (디렉토리 분류 = type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageType {
    /// 실체 (사람, 프로젝트, 도구) — entity/*
    Entity,
    /// 개념 (방법론, 패턴) — concept/*
    Concept,
    /// 비교 (A vs B) — comparison/*
    Comparison,
    /// 사용자 정의 (확장)
    Other,
}

impl PageType {
    /// 디렉토리 이름 (snake-case).
    pub fn as_dir(self) -> &'static str {
        match self {
            PageType::Entity => "entity",
            PageType::Concept => "concept",
            PageType::Comparison => "comparison",
            PageType::Other => "other",
        }
    }
}

impl fmt::Display for PageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_dir())
    }
}

impl FromStr for PageType {
    type Err = WikiError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "entity" => Ok(PageType::Entity),
            "concept" => Ok(PageType::Concept),
            "comparison" => Ok(PageType::Comparison),
            "other" => Ok(PageType::Other),
            other => Err(WikiError::UnknownPageType(other.to_string())),
        }
    }
}

/// 페이지 id (`{type}/{slug}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageId(String);

impl PageId {
    /// type + slug에서 id 구성. slug는 빈 문자열 금지, `/` 포함 금지.
    pub fn new(page_type: PageType, slug: &str) -> Result<Self, WikiError> {
        let slug = slug.trim();
        if slug.is_empty() || slug.contains('/') || slug.contains('\\') {
            return Err(WikiError::InvalidPageId(slug.to_string()));
        }
        Ok(PageId(format!("{}/{}", page_type.as_dir(), slug)))
    }

    /// 파싱: `entity/foo` → (Entity, "foo")
    pub fn parse(s: &str) -> Result<(PageType, String), WikiError> {
        let s = s.trim();
        let (ty, slug) = s
            .split_once('/')
            .ok_or_else(|| WikiError::InvalidPageId(s.to_string()))?;
        if slug.is_empty() {
            return Err(WikiError::InvalidPageId(s.to_string()));
        }
        let page_type = PageType::from_str(ty)?;
        Ok((page_type, slug.to_string()))
    }

    /// 문자열로.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 디스크 상대 경로 (`entity/사용자.md`).
    pub fn file_path(&self) -> String {
        format!("{}.md", self.0)
    }
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PageId {
    type Err = WikiError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 검증을 위해 parse를 거친 후 정규화된 id 반환
        let (ty, slug) = PageId::parse(s)?;
        PageId::new(ty, &slug)
    }
}

/// 관련 페이지 링크.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Related {
    /// 대상 페이지 id.
    pub to: PageId,
    /// 관계 사유 (선택).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Wiki frontmatter — YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    /// 페이지 id.
    pub id: String,
    /// 페이지 type.
    pub r#type: String,
    /// 생성 시각 (KST tz, ISO 8601).
    pub created: DateTime<Utc>,
    /// 갱신 시각.
    pub updated: DateTime<Utc>,
    /// 관련 페이지 id 배열.
    #[serde(default)]
    pub related: Vec<String>,
    /// 원천 메시지·에피소드 ref 배열 (예: `msg:abc`, `episode:def`).
    #[serde(default)]
    pub source_refs: Vec<String>,
    /// content_hash로 임베딩 재계산 여부 결정.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_hash: Option<String>,
}

/// 위키 페이지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// 페이지 id.
    pub id: PageId,
    /// type.
    pub page_type: PageType,
    /// 제목 (본문 첫 `# ` 헤더 또는 slug).
    pub title: String,
    /// 본문 (frontmatter 제외).
    pub body: String,
    /// 관련 페이지.
    pub related: Vec<Related>,
    /// 원천 ref.
    pub source_refs: Vec<String>,
    /// 생성 시각.
    pub created_at: DateTime<Utc>,
    /// 갱신 시각.
    pub updated_at: DateTime<Utc>,
    /// 콘텐츠 hash (body 기준).
    pub content_hash: String,
    /// 임베딩 hash (body 기준; 변경 시 재임베딩).
    pub embedding_hash: String,
}

impl Page {
    /// 신규 페이지 생성 (created=updated=now).
    pub fn new(id: PageId, page_type: PageType, title: String, body: String) -> Self {
        let now = Utc::now();
        let body_hash = content_hash(&body);
        Self {
            id,
            page_type,
            title,
            body,
            related: Vec::new(),
            source_refs: Vec::new(),
            created_at: now,
            updated_at: now,
            content_hash: body_hash.clone(),
            embedding_hash: body_hash,
        }
    }

    /// 페이지를 disk markdown으로 직렬화 (frontmatter + body).
    pub fn to_markdown(&self) -> Result<String, WikiError> {
        let fm = Frontmatter {
            id: self.id.to_string(),
            r#type: self.page_type.to_string(),
            created: self.created_at,
            updated: self.updated_at,
            related: self.related.iter().map(|r| r.to.to_string()).collect(),
            source_refs: self.source_refs.clone(),
            embedding_hash: Some(self.embedding_hash.clone()),
        };
        let yaml = serde_yaml_simple(&fm)?;
        Ok(format!("---\n{}---\n\n{}\n", yaml, self.body.trim_end()))
    }

    /// disk markdown에서 Page 복원.
    pub fn from_markdown(source: &str) -> Result<Self, WikiError> {
        let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
        let parsed = matter.parse(source);
        let body = parsed.content.clone();

        let raw_data = parsed.data.ok_or_else(|| {
            WikiError::Frontmatter("missing frontmatter (--- ... ---)".to_string())
        })?;

        // gray_matter::Pod → serde_json::Value 변환은 간단치 않으므로
        // 직접 필드 추출.
        let fm: Frontmatter = raw_data
            .deserialize()
            .map_err(|e| WikiError::Frontmatter(format!("yaml: {e}")))?;

        let id = PageId::from_str(&fm.id)?;
        let page_type = PageType::from_str(&fm.r#type)?;
        let title = extract_title(&body).unwrap_or_else(|| {
            // fallback: id의 slug
            id.as_str()
                .split_once('/')
                .map(|(_, slug)| slug.to_string())
                .unwrap_or_else(|| id.to_string())
        });

        let related = fm
            .related
            .iter()
            .filter_map(|s| PageId::from_str(s).ok())
            .map(|to| Related { to, reason: None })
            .collect();

        let body_hash = content_hash(&body);
        Ok(Self {
            id,
            page_type,
            title,
            body: body.trim_end().to_string(),
            related,
            source_refs: fm.source_refs,
            created_at: fm.created,
            updated_at: fm.updated,
            content_hash: body_hash.clone(),
            embedding_hash: fm.embedding_hash.unwrap_or(body_hash),
        })
    }
}

/// `# Title` 첫 헤더 추출.
fn extract_title(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// gray_matter는 YAML 출력 미지원 → 수동 직렬화 (단순 키만 사용).
fn serde_yaml_simple(fm: &Frontmatter) -> Result<String, WikiError> {
    let mut out = String::new();
    out.push_str(&format!("id: {}\n", fm.id));
    out.push_str(&format!("type: {}\n", fm.r#type));
    out.push_str(&format!("created: {}\n", fm.created.to_rfc3339()));
    out.push_str(&format!("updated: {}\n", fm.updated.to_rfc3339()));
    if !fm.related.is_empty() {
        out.push_str("related:\n");
        for r in &fm.related {
            out.push_str(&format!("  - {}\n", r));
        }
    }
    if !fm.source_refs.is_empty() {
        out.push_str("source_refs:\n");
        for r in &fm.source_refs {
            out.push_str(&format!("  - {}\n", r));
        }
    }
    if let Some(eh) = &fm.embedding_hash {
        out.push_str(&format!("embedding_hash: {}\n", eh));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_id_roundtrip() {
        let id = PageId::new(PageType::Entity, "사용자").unwrap();
        assert_eq!(id.as_str(), "entity/사용자");
        let parsed = PageId::from_str("entity/사용자").unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn page_id_rejects_slash_in_slug() {
        assert!(PageId::new(PageType::Entity, "foo/bar").is_err());
    }

    #[test]
    fn page_id_rejects_empty_slug() {
        assert!(PageId::new(PageType::Entity, "").is_err());
        assert!(PageId::from_str("entity/").is_err());
        assert!(PageId::from_str("entity").is_err());
    }

    #[test]
    fn markdown_roundtrip() {
        let mut page = Page::new(
            PageId::new(PageType::Concept, "test").unwrap(),
            PageType::Concept,
            "Test Title".to_string(),
            "# Test Title\n\nbody content here".to_string(),
        );
        page.source_refs.push("msg:abc".to_string());
        page.related.push(Related {
            to: PageId::new(PageType::Entity, "other").unwrap(),
            reason: None,
        });

        let md = page.to_markdown().unwrap();
        let parsed = Page::from_markdown(&md).unwrap();
        assert_eq!(parsed.id, page.id);
        assert_eq!(parsed.title, "Test Title");
        assert_eq!(parsed.source_refs, page.source_refs);
        assert_eq!(parsed.related.len(), 1);
        assert_eq!(parsed.related[0].to, page.related[0].to);
    }
}
