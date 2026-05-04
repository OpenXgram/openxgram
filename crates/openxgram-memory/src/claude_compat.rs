//! Claude-compatible memory export/import (PRD-MEM-CLAUDE-COMPAT).
//!
//! Anthropic 공식 메모리 export/import 형식과 호환:
//! - https://support.claude.com/en/articles/12123587-import-and-export-your-memory-from-claude
//!
//! 5 카테고리: Instructions / Identity / Career / Projects / Preferences
//! 형식: 카테고리별 헤더 + `[YYYY-MM-DD] - content` 라인, oldest first.
//! 출력은 단일 code block 으로 wrap.
//!
//! 매핑 (OpenXgram → Claude 카테고리):
//! - L2 memory kind=rule        → Instructions
//! - L4 traits 일부 (identity)  → Identity
//! - L2 memory kind=fact        → Identity / Career / Projects (휴리스틱)
//! - L2 memory kind=decision    → Projects
//! - L2 memory kind=reference   → Projects / Career
//! - L4 traits + L3 patterns    → Preferences
//!
//! 절대 규칙:
//! - "Do not summarize, group, or omit any entries" — 원문 보존
//! - 시간 unknown 시 `[unknown]`
//! - 단일 code block 으로 wrap (호출자 stdout 으로 출력 시)

use chrono::{DateTime, FixedOffset};
use openxgram_db::Db;
use serde::{Deserialize, Serialize};

use crate::memory::{Memory, MemoryKind, MemoryStore};
use crate::traits::{AgentTrait, TraitSource, TraitStore};
use crate::Result;

/// Claude memory export 카테고리 (5개, 사용자 지정 순서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeCategory {
    Instructions,
    Identity,
    Career,
    Projects,
    Preferences,
}

impl ClaudeCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instructions => "Instructions",
            Self::Identity => "Identity",
            Self::Career => "Career",
            Self::Projects => "Projects",
            Self::Preferences => "Preferences",
        }
    }

    pub fn parse_header(s: &str) -> Option<Self> {
        let trimmed = s
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_end_matches(':')
            .trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "instructions" => Some(Self::Instructions),
            "identity" => Some(Self::Identity),
            "career" => Some(Self::Career),
            "projects" => Some(Self::Projects),
            "preferences" => Some(Self::Preferences),
            _ => None,
        }
    }

    /// Export 시 카테고리 출력 순서 (사용자 지정).
    pub fn ordered() -> [Self; 5] {
        [
            Self::Instructions,
            Self::Identity,
            Self::Career,
            Self::Projects,
            Self::Preferences,
        ]
    }
}

/// 한 줄 entry — `[YYYY-MM-DD] - content`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeEntry {
    /// 저장 일자. None 이면 `[unknown]`.
    pub date: Option<chrono::NaiveDate>,
    pub content: String,
}

impl ClaudeEntry {
    pub fn render(&self) -> String {
        let date_str = self
            .date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        format!("[{date_str}] - {}", self.content)
    }
}

/// L2 memory → Claude 카테고리 매핑 (휴리스틱 + kind 기반).
fn classify_memory(m: &Memory) -> ClaudeCategory {
    match m.kind {
        MemoryKind::Rule => ClaudeCategory::Instructions,
        MemoryKind::Decision => ClaudeCategory::Projects,
        MemoryKind::Reference => {
            if looks_like_career(&m.content) {
                ClaudeCategory::Career
            } else {
                ClaudeCategory::Projects
            }
        }
        MemoryKind::Fact => {
            if looks_like_career(&m.content) {
                ClaudeCategory::Career
            } else if looks_like_identity(&m.content) {
                ClaudeCategory::Identity
            } else {
                ClaudeCategory::Projects
            }
        }
    }
}

/// 휴리스틱 — career 키워드 (회사·역할·기술 도구).
fn looks_like_career(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [
        "company", "role", "engineer", "developer", "ceo", "cto", "founder",
        "startup", "team lead", "회사", "직장", "직책", "역할", "팀장", "엔지니어",
        "개발자", "창업",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

/// 휴리스틱 — identity 키워드 (이름·나이·지역·언어).
fn looks_like_identity(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [
        "name is", "i am ", "이름은", "나이", "지역", "사는 곳", "고향", "거주",
        "language", "언어", "korean", "english",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

/// trait → Identity 또는 Preferences (source 기반).
fn classify_trait(t: &AgentTrait) -> ClaudeCategory {
    if t.name.to_ascii_lowercase().contains("identity") || looks_like_identity(&t.value) {
        ClaudeCategory::Identity
    } else {
        ClaudeCategory::Preferences
    }
}

/// 단일 entry 묶음 (카테고리별 정렬된 ClaudeEntry 목록).
#[derive(Debug, Default)]
pub struct ClaudeExport {
    pub buckets: std::collections::BTreeMap<ClaudeCategory, Vec<ClaudeEntry>>,
}

impl ClaudeExport {
    fn push(&mut self, cat: ClaudeCategory, entry: ClaudeEntry) {
        self.buckets.entry(cat).or_default().push(entry);
    }

    /// oldest first 정렬 (Claude 공식 권장).
    fn sort_each(&mut self) {
        for entries in self.buckets.values_mut() {
            entries.sort_by(|a, b| match (a.date, b.date) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
        }
    }

    /// 단일 code block 형태로 markdown 출력.
    pub fn render_markdown(&self) -> String {
        let mut out = String::from("```\n");
        for cat in ClaudeCategory::ordered() {
            let entries = match self.buckets.get(&cat) {
                Some(e) if !e.is_empty() => e,
                _ => continue,
            };
            out.push_str(&format!("## {}\n\n", cat.as_str()));
            for e in entries {
                out.push_str(&e.render());
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("```\n");
        out
    }
}

/// DB 의 L2 memories + L4 traits 를 Claude 호환 export 로.
pub fn export_claude(db: &mut Db) -> Result<ClaudeExport> {
    let mut export = ClaudeExport::default();

    // L2 memories — kind 별 list
    for kind in [
        MemoryKind::Fact,
        MemoryKind::Decision,
        MemoryKind::Reference,
        MemoryKind::Rule,
    ] {
        let memories = MemoryStore::new(db).list_by_kind(kind)?;
        for m in memories {
            let cat = classify_memory(&m);
            export.push(
                cat,
                ClaudeEntry {
                    date: Some(m.created_at.naive_local().date()),
                    content: m.content,
                },
            );
        }
    }

    // L4 traits
    let traits = TraitStore::new(db).list()?;
    for t in traits {
        let cat = classify_trait(&t);
        let content = format!("{}: {}", t.name, t.value);
        export.push(
            cat,
            ClaudeEntry {
                date: Some(t.created_at.naive_local().date()),
                content,
            },
        );
    }

    export.sort_each();
    Ok(export)
}

/// Claude 호환 export 텍스트를 파싱 → 카테고리별 entry 목록.
pub fn parse_claude(text: &str) -> ClaudeExport {
    let mut export = ClaudeExport::default();
    let stripped = text
        .trim()
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut current: Option<ClaudeCategory> = None;
    for raw in stripped.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(cat) = ClaudeCategory::parse_header(line) {
            current = Some(cat);
            continue;
        }
        if let Some(entry) = parse_entry_line(line) {
            if let Some(cat) = current {
                export.push(cat, entry);
            }
        }
    }
    export.sort_each();
    export
}

fn parse_entry_line(line: &str) -> Option<ClaudeEntry> {
    let line = line.trim().trim_start_matches('-').trim();
    if !line.starts_with('[') {
        return None;
    }
    let close = line.find(']')?;
    let date_str = &line[1..close];
    let rest = line[close + 1..].trim_start_matches(' ').trim_start_matches('-').trim();
    let date = if date_str == "unknown" {
        None
    } else {
        chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
    };
    Some(ClaudeEntry {
        date,
        content: rest.to_string(),
    })
}

/// Import — 카테고리별 entry 를 L2 memory 또는 L4 trait 로 저장.
/// session_id None → 글로벌 (kind=rule/reference 가 자연스럽게 cross-session).
pub fn import_claude(db: &mut Db, parsed: &ClaudeExport) -> Result<ClaudeImportSummary> {
    let mut summary = ClaudeImportSummary::default();

    for (&cat, entries) in &parsed.buckets {
        let kind = category_to_memory_kind(cat);
        for e in entries {
            match cat {
                ClaudeCategory::Identity | ClaudeCategory::Preferences => {
                    // L4 trait 로 저장 (insert_or_update — 같은 name 이면 갱신)
                    let name = derive_trait_name(&e.content, cat);
                    TraitStore::new(db).insert_or_update(
                        &name,
                        &e.content,
                        TraitSource::Manual,
                        &[format!("claude-import:{}", cat.as_str().to_lowercase())],
                    )?;
                    summary.traits_inserted += 1;
                }
                _ => {
                    // L2 memory 로 저장 (session_id None = 글로벌)
                    let _inserted = MemoryStore::new(db).insert(None, kind, &e.content)?;
                    summary.memories_inserted += 1;
                }
            }
        }
    }
    Ok(summary)
}

#[derive(Debug, Default, Clone)]
pub struct ClaudeImportSummary {
    pub memories_inserted: usize,
    pub traits_inserted: usize,
}

fn category_to_memory_kind(cat: ClaudeCategory) -> MemoryKind {
    match cat {
        ClaudeCategory::Instructions => MemoryKind::Rule,
        ClaudeCategory::Career | ClaudeCategory::Identity => MemoryKind::Fact,
        ClaudeCategory::Projects => MemoryKind::Decision,
        ClaudeCategory::Preferences => MemoryKind::Reference,
    }
}

fn derive_trait_name(content: &str, cat: ClaudeCategory) -> String {
    // 첫 8단어 추출, 비ASCII 도 보존, 길이 64 제한
    let head: String = content.chars().take(64).collect();
    format!("{}:{}", cat.as_str().to_lowercase(), head)
}

/// 공식 권장 prompt — `xgram memory export-prompt` 출력용.
pub const CLAUDE_EXPORT_PROMPT: &str = "Export all of my stored memories and any context you've learned about me from past conversations. Preserve my words verbatim where possible, especially for instructions and preferences.\n\n## Categories (output in this order):\n\n1. **Instructions**: Rules I've explicitly asked you to follow going forward — tone, format, style, \"always do X\", \"never do Y\", and corrections to your behavior. Only include rules from stored memories, not from conversations.\n\n2. **Identity**: Name, age, location, education, family, relationships, languages, and personal interests.\n\n3. **Career**: Current and past roles, companies, and general skill areas.\n\n4. **Projects**: Projects I meaningfully built or committed to. Ideally ONE entry per project. Include what it does, current status, and any key decisions. Use the project name or a short descriptor as the first words of the entry.\n\n5. **Preferences**: Opinions, tastes, and working-style preferences that apply broadly.\n\n## Format:\n\nUse section headers for each category. Within each category, list one entry per line, sorted by oldest date first. Format each line as:\n\n[YYYY-MM-DD] - Entry content here.\n\nIf no date is known, use [unknown] instead.\n\n## Output:\n- Wrap the entire export in a single code block for easy copying.\n- After the code block, state whether this is the complete set or if more remain.\n";

#[allow(dead_code)]
fn _ts_to_date(ts: DateTime<FixedOffset>) -> chrono::NaiveDate {
    ts.naive_local().date()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn entry_render_with_date() {
        let e = ClaudeEntry {
            date: NaiveDate::from_ymd_opt(2026, 5, 4),
            content: "Use spaces over tabs.".into(),
        };
        assert_eq!(e.render(), "[2026-05-04] - Use spaces over tabs.");
    }

    #[test]
    fn entry_render_unknown_date() {
        let e = ClaudeEntry {
            date: None,
            content: "Always reply in Korean.".into(),
        };
        assert_eq!(e.render(), "[unknown] - Always reply in Korean.");
    }

    #[test]
    fn parse_round_trip() {
        let input = r#"```
## Instructions

[2026-05-04] - Always reply in Korean.
[2026-04-15] - Never auto-deploy without explicit master approval.

## Preferences

[2026-04-22] - Prefers minimal libraries over frameworks.
[unknown] - Likes dark themes.
```"#;
        let parsed = parse_claude(input);
        let inst = parsed.buckets.get(&ClaudeCategory::Instructions).unwrap();
        assert_eq!(inst.len(), 2);
        // sorted oldest first
        assert_eq!(inst[0].content, "Never auto-deploy without explicit master approval.");
        assert_eq!(inst[1].content, "Always reply in Korean.");
        let pref = parsed.buckets.get(&ClaudeCategory::Preferences).unwrap();
        assert_eq!(pref.len(), 2);
        assert!(pref[1].date.is_none(), "unknown 은 마지막");
    }

    #[test]
    fn category_parse_header_variants() {
        assert_eq!(
            ClaudeCategory::parse_header("## Instructions"),
            Some(ClaudeCategory::Instructions)
        );
        assert_eq!(
            ClaudeCategory::parse_header("Identity:"),
            Some(ClaudeCategory::Identity)
        );
        assert_eq!(
            ClaudeCategory::parse_header("# preferences"),
            Some(ClaudeCategory::Preferences)
        );
        assert_eq!(ClaudeCategory::parse_header("random text"), None);
    }

    #[test]
    fn classify_memory_rule_to_instructions() {
        let m = Memory {
            id: "1".into(),
            session_id: None,
            kind: MemoryKind::Rule,
            content: "always KST timezone".into(),
            pinned: true,
            importance: 1.0,
            access_count: 0,
            created_at: chrono::Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()),
            last_accessed: chrono::Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()),
        };
        assert_eq!(classify_memory(&m), ClaudeCategory::Instructions);
    }

    #[test]
    fn classify_memory_career_keyword() {
        let m = Memory {
            id: "1".into(),
            session_id: None,
            kind: MemoryKind::Fact,
            content: "I am the CTO of Acme Inc.".into(),
            pinned: false,
            importance: 0.5,
            access_count: 0,
            created_at: chrono::Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()),
            last_accessed: chrono::Utc::now().with_timezone(&FixedOffset::east_opt(9*3600).unwrap()),
        };
        assert_eq!(classify_memory(&m), ClaudeCategory::Career);
    }

    #[test]
    fn render_markdown_wraps_code_block() {
        let mut export = ClaudeExport::default();
        export.push(
            ClaudeCategory::Instructions,
            ClaudeEntry {
                date: NaiveDate::from_ymd_opt(2026, 5, 4),
                content: "Always reply in Korean.".into(),
            },
        );
        let out = export.render_markdown();
        assert!(out.starts_with("```\n"));
        assert!(out.ends_with("```\n"));
        assert!(out.contains("## Instructions"));
        assert!(out.contains("[2026-05-04] - Always reply in Korean."));
    }

    #[test]
    fn parse_entry_line_handles_dash_prefix() {
        let e = parse_entry_line("- [2026-05-04] - foo bar").unwrap();
        assert_eq!(e.content, "foo bar");
        assert!(e.date.is_some());
    }

    #[test]
    fn export_prompt_contains_5_categories() {
        for c in ClaudeCategory::ordered() {
            assert!(
                CLAUDE_EXPORT_PROMPT.contains(c.as_str()),
                "prompt missing {}",
                c.as_str()
            );
        }
    }
}
