//! step 5/6/8 — AI 앱 export 파일을 OpenXgram session + messages 로 import.
//!
//! 지원 형식:
//!   - chatgpt      : ChatGPT "Export Data" zip 안의 `conversations.json`
//!   - gemini       : Google Takeout `Bard/MyActivity.json` 또는 `Gemini/MyActivity.json`
//!   - claude-code  : `~/.claude/projects/<proj>/sessions/<id>.jsonl`
//!
//! 결과:
//!   - 새 session 1개 (title = export 의 conversation 제목, 또는 파일명)
//!   - 메시지 N개 (sender = "user" / "ChatGPT" / "Gemini" / "Claude")
//!   - 모두 같은 conversation_id (한 import = 한 thread)

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Chatgpt,
    Gemini,
    ClaudeCode,
}

impl ImportFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "chatgpt" => Ok(Self::Chatgpt),
            "gemini" => Ok(Self::Gemini),
            "claude-code" | "claudecode" => Ok(Self::ClaudeCode),
            other => Err(anyhow!(
                "format 은 chatgpt | gemini | claude-code (got: {other})"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub session_id: String,
    pub title: String,
    pub messages_inserted: usize,
}

pub fn run_import_app(
    data_dir: &Path,
    file: &Path,
    format: ImportFormat,
    title_override: Option<&str>,
) -> Result<ImportSummary> {
    if !file.exists() {
        bail!("파일 없음: {}", file.display());
    }
    let raw = std::fs::read_to_string(file)
        .with_context(|| format!("파일 읽기 실패: {}", file.display()))?;
    let (default_title, sender_label, turns) = match format {
        ImportFormat::Chatgpt => parse_chatgpt(&raw)?,
        ImportFormat::Gemini => parse_gemini(&raw)?,
        ImportFormat::ClaudeCode => parse_claude_code(&raw)?,
    };
    let title = title_override
        .map(str::to_string)
        .unwrap_or(default_title)
        .trim()
        .to_string();
    let title = if title.is_empty() {
        format!("import-{}", file.file_name().and_then(|s| s.to_str()).unwrap_or("untitled"))
    } else {
        title
    };

    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;
    let embedder = default_embedder()?;
    let session = SessionStore::new(&mut db)
        .ensure_by_title(&title, "imported")
        .with_context(|| format!("session ensure: {title}"))?;
    let session_id = session.id.clone();

    // 한 import = 한 conversation_id (모든 메시지 같은 thread)
    let mut store = MessageStore::new(&mut db, embedder.as_ref());
    let mut conv_id: Option<String> = None;
    let mut count = 0usize;
    for turn in turns {
        let sender = match turn.role {
            Role::User => "user".to_string(),
            Role::Assistant => sender_label.to_string(),
            Role::System => "system".to_string(),
        };
        let inserted = store.insert(
            &session_id,
            &sender,
            &turn.text,
            sender_label,
            conv_id.as_deref(),
        )?;
        if conv_id.is_none() {
            conv_id = Some(inserted.conversation_id.clone());
        }
        count += 1;
    }

    Ok(ImportSummary {
        session_id,
        title,
        messages_inserted: count,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
struct Turn {
    role: Role,
    text: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// ChatGPT — conversations.json (Export Data zip 의 한 파일)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatgptConversation {
    #[serde(default)]
    title: Option<String>,
    mapping: std::collections::HashMap<String, ChatgptNode>,
    #[serde(default)]
    current_node: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatgptNode {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    message: Option<ChatgptMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatgptMessage {
    #[serde(default)]
    author: Option<ChatgptAuthor>,
    #[serde(default)]
    content: Option<ChatgptContent>,
}

#[derive(Debug, Deserialize)]
struct ChatgptAuthor {
    role: String,
}

#[derive(Debug, Deserialize)]
struct ChatgptContent {
    #[serde(default)]
    parts: Option<Vec<serde_json::Value>>,
}

fn parse_chatgpt(raw: &str) -> Result<(String, &'static str, Vec<Turn>)> {
    // 단일 conversation 또는 array. array 면 첫 번째만.
    let v: serde_json::Value = serde_json::from_str(raw).context("ChatGPT JSON 파싱")?;
    let conv: ChatgptConversation = match v {
        serde_json::Value::Array(arr) => {
            let first = arr.into_iter().next().ok_or_else(|| anyhow!("빈 conversations array"))?;
            serde_json::from_value(first)?
        }
        other => serde_json::from_value(other)?,
    };
    let title = conv.title.clone().unwrap_or_default();

    // mapping 은 트리. current_node 부터 parent 따라 root 까지 거꾸로 — 그 후 reverse 로 시간순.
    let mut chain: Vec<String> = Vec::new();
    let mut cursor = conv.current_node.clone();
    while let Some(id) = cursor {
        let node = match conv.mapping.get(&id) {
            Some(n) => n,
            None => break,
        };
        chain.push(id.clone());
        cursor = node.parent.clone();
    }
    chain.reverse();

    let mut turns = Vec::new();
    for node_id in chain {
        let Some(node) = conv.mapping.get(&node_id) else {
            continue;
        };
        let Some(msg) = node.message.as_ref() else {
            continue;
        };
        let role = match msg.author.as_ref().map(|a| a.role.as_str()) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("system") => Role::System,
            _ => continue,
        };
        let text = msg
            .content
            .as_ref()
            .and_then(|c| c.parts.as_ref())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| {
                        if let Some(s) = p.as_str() {
                            Some(s.to_string())
                        } else if let Some(obj) = p.as_object() {
                            obj.get("text").and_then(|t| t.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        turns.push(Turn { role, text });
    }

    Ok((title, "ChatGPT", turns))
}

// ─────────────────────────────────────────────────────────────────────────────
// Gemini — Google Takeout MyActivity.json
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GeminiActivity {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    products: Option<Vec<String>>,
    /// 사용자 prompt (또는 응답 일부) — Takeout 은 entry 별 단위
    #[serde(default)]
    subtitles: Option<Vec<GeminiSubtitle>>,
    #[serde(default)]
    time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiSubtitle {
    #[serde(default)]
    name: Option<String>,
}

fn parse_gemini(raw: &str) -> Result<(String, &'static str, Vec<Turn>)> {
    let v: Vec<GeminiActivity> = serde_json::from_str(raw).context("Gemini JSON 파싱")?;
    let mut turns = Vec::new();
    for entry in v {
        // Takeout 의 "title" 은 보통 사용자 prompt + ":" 형식. "Searched for: ..." / "Asked: ..." 같은 prefix 흔함.
        let raw_title = entry.title.unwrap_or_default();
        let body = raw_title
            .trim_start_matches("Asked: ")
            .trim_start_matches("Searched for: ")
            .trim_start_matches("Asked Bard: ")
            .trim_start_matches("Asked Gemini: ")
            .trim()
            .to_string();
        if body.is_empty() {
            continue;
        }
        // Takeout 은 user prompt 만 보존 (응답 미포함). subtitles 가 있으면 그것도 system note.
        turns.push(Turn {
            role: Role::User,
            text: body,
        });
        if let Some(subs) = entry.subtitles {
            for s in subs {
                if let Some(n) = s.name {
                    if !n.trim().is_empty() {
                        turns.push(Turn {
                            role: Role::System,
                            text: n,
                        });
                    }
                }
            }
        }
    }
    Ok(("Gemini activity import".into(), "Gemini", turns))
}

// ─────────────────────────────────────────────────────────────────────────────
// Claude Code — JSONL session log
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ClaudeCodeLine {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    message: Option<ClaudeCodeMessage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeCodeMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<serde_json::Value>,
}

fn parse_claude_code(raw: &str) -> Result<(String, &'static str, Vec<Turn>)> {
    let mut turns = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let parsed: ClaudeCodeLine = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[import][claude-code] line {} skip: {e}", i + 1);
                continue;
            }
        };
        let role = match parsed
            .kind
            .as_deref()
            .or(parsed.message.as_ref().and_then(|m| m.role.as_deref()))
        {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("system") => Role::System,
            _ => continue,
        };
        let text = parsed
            .message
            .as_ref()
            .and_then(|m| m.content.as_ref())
            .map(|c| extract_text(c))
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        turns.push(Turn { role, text });
    }
    Ok(("Claude Code session import".into(), "Claude", turns))
}

fn extract_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.into();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|p| {
                p.as_object()
                    .and_then(|o| o.get("text"))
                    .and_then(|t| t.as_str())
                    .map(String::from)
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_core::paths::manifest_path;
    use tempfile::tempdir;

    fn open_test_dir() -> tempfile::TempDir {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();
        tmp
    }

    #[test]
    fn format_parse_round_trip() {
        assert_eq!(ImportFormat::parse("chatgpt").unwrap(), ImportFormat::Chatgpt);
        assert_eq!(ImportFormat::parse("gemini").unwrap(), ImportFormat::Gemini);
        assert_eq!(ImportFormat::parse("claude-code").unwrap(), ImportFormat::ClaudeCode);
        assert!(ImportFormat::parse("invalid").is_err());
    }

    #[test]
    fn chatgpt_parse_minimal_conversation() {
        let raw = r#"
        {
            "title": "test conversation",
            "mapping": {
                "n1": {"parent": null, "message": {"author": {"role": "user"}, "content": {"parts": ["안녕"]}}},
                "n2": {"parent": "n1", "message": {"author": {"role": "assistant"}, "content": {"parts": ["반가워"]}}}
            },
            "current_node": "n2"
        }
        "#;
        let (title, label, turns) = parse_chatgpt(raw).unwrap();
        assert_eq!(title, "test conversation");
        assert_eq!(label, "ChatGPT");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text, "안녕");
        assert_eq!(turns[1].text, "반가워");
        assert_eq!(turns[0].role, Role::User);
        assert_eq!(turns[1].role, Role::Assistant);
    }

    #[test]
    fn chatgpt_parse_array_form() {
        let raw = r#"
        [
            {
                "title": "first",
                "mapping": {"n1": {"parent": null, "message": {"author": {"role": "user"}, "content": {"parts": ["hi"]}}}},
                "current_node": "n1"
            }
        ]
        "#;
        let (title, _, turns) = parse_chatgpt(raw).unwrap();
        assert_eq!(title, "first");
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn gemini_parse_takeout_entries() {
        let raw = r#"
        [
            {"title": "Asked Gemini: 5월 시장 분석해줘", "products": ["Gemini"], "time": "2026-05-10T14:00:00Z"},
            {"title": "Asked: 보고서 형식으로 정리해줘"}
        ]
        "#;
        let (_, label, turns) = parse_gemini(raw).unwrap();
        assert_eq!(label, "Gemini");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text, "5월 시장 분석해줘");
        assert_eq!(turns[1].text, "보고서 형식으로 정리해줘");
    }

    #[test]
    fn claude_code_parse_jsonl() {
        let raw = r#"{"type":"user","message":{"role":"user","content":"이 코드 검증"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"검증 결과: OK"}]}}
{"random":"line"}
"#;
        let (_, label, turns) = parse_claude_code(raw).unwrap();
        assert_eq!(label, "Claude");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text, "이 코드 검증");
        assert_eq!(turns[1].text, "검증 결과: OK");
    }

    #[test]
    fn import_chatgpt_into_session_inserts_messages() {
        let tmp = open_test_dir();
        let dir = tmp.path();
        let raw = r#"
        {
            "title": "demo",
            "mapping": {
                "n1": {"parent": null, "message": {"author": {"role": "user"}, "content": {"parts": ["hi"]}}},
                "n2": {"parent": "n1", "message": {"author": {"role": "assistant"}, "content": {"parts": ["hello"]}}}
            },
            "current_node": "n2"
        }
        "#;
        let file = dir.join("c.json");
        std::fs::write(&file, raw).unwrap();
        // init DB schema
        {
            let mut db = Db::open(DbConfig {
                path: db_path(dir),
                ..Default::default()
            })
            .unwrap();
            db.migrate().unwrap();
        }
        let summary = run_import_app(dir, &file, ImportFormat::Chatgpt, None).unwrap();
        assert_eq!(summary.title, "demo");
        assert_eq!(summary.messages_inserted, 2);
    }

    #[test]
    fn import_rejects_missing_file() {
        let tmp = open_test_dir();
        let dir = tmp.path();
        let res = run_import_app(
            dir,
            &dir.join("nonexistent.json"),
            ImportFormat::Chatgpt,
            None,
        );
        assert!(res.is_err());
    }

    #[test]
    fn import_uses_title_override() {
        let tmp = open_test_dir();
        let dir = tmp.path();
        let raw = r#"{"title":"orig","mapping":{"n":{"parent":null,"message":{"author":{"role":"user"},"content":{"parts":["x"]}}}},"current_node":"n"}"#;
        let f = dir.join("c.json");
        std::fs::write(&f, raw).unwrap();
        {
            let mut db = Db::open(DbConfig { path: db_path(dir), ..Default::default() }).unwrap();
            db.migrate().unwrap();
        }
        let s = run_import_app(dir, &f, ImportFormat::Chatgpt, Some("override-title")).unwrap();
        assert_eq!(s.title, "override-title");
    }
}
