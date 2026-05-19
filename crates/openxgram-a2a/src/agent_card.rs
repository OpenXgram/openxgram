//! AgentCard — A2A 표준 메타데이터 (스킬·인증·기능).
//!
//! 정본: https://a2a-protocol.org/latest/specification/#agent-card
//!
//! 외부 A2A 에이전트가 `/.well-known/agent-card.json`으로 노출하는 JSON.

use serde::{Deserialize, Serialize};

/// A2A AgentCard — 에이전트 메타데이터.
///
/// 외부 에이전트의 `/.well-known/agent-card.json` 에서 가져옴.
/// 표준 필드만 강타입화하고 그 외는 `extra` 로 보존 (forward-compat).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// 에이전트 표시명.
    pub name: String,

    /// 한 문장 설명.
    pub description: String,

    /// 에이전트 base URL (JSON-RPC endpoint).
    pub url: String,

    /// 에이전트 버전 (semver 또는 자유 문자열).
    pub version: String,

    /// 인증 스킴 (bearer / oauth2 / none / 커스텀).
    #[serde(default)]
    pub authentication: Authentication,

    /// 노출하는 skill 목록.
    #[serde(default)]
    pub skills: Vec<AgentSkill>,

    /// 선택적 기능 플래그.
    #[serde(default)]
    pub capabilities: AgentCapabilities,

    /// 표준 외 필드 보존 (forward-compat).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 인증 스킴 정의.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Authentication {
    /// 지원 스킴 ("bearer", "oauth2", "none" 등).
    #[serde(default)]
    pub schemes: Vec<String>,

    /// OAuth2/OIDC 등 추가 메타.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 에이전트 skill (호출 가능 단위).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// 호출 시 사용할 id (예: `translate`).
    pub id: String,

    /// 표시명.
    pub name: String,

    /// 설명.
    #[serde(default)]
    pub description: String,

    /// 지원 입력 모달리티 ("text" / "data" / "file").
    #[serde(default, rename = "inputModes", alias = "input_modes")]
    pub input_modes: Vec<String>,

    /// 지원 출력 모달리티.
    #[serde(default, rename = "outputModes", alias = "output_modes")]
    pub output_modes: Vec<String>,

    /// 표준 외 필드.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 선택 기능.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// SSE 스트리밍 지원 여부.
    #[serde(default)]
    pub streaming: bool,

    /// push notification 지원 여부.
    #[serde(default, rename = "pushNotifications", alias = "push_notifications")]
    pub push_notifications: bool,

    /// 표준 외 capability.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "name": "Translation Agent",
        "description": "Translates text between languages",
        "url": "https://translate.example.com/agent",
        "version": "1.0.0",
        "authentication": { "schemes": ["bearer"] },
        "skills": [{
            "id": "translate",
            "name": "Translate text",
            "description": "Translate text from one language to another",
            "inputModes": ["text"],
            "outputModes": ["text"]
        }],
        "capabilities": { "streaming": true, "pushNotifications": false }
    }"#;

    #[test]
    fn parses_canonical_fixture() {
        let card: AgentCard = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(card.name, "Translation Agent");
        assert_eq!(card.url, "https://translate.example.com/agent");
        assert_eq!(card.authentication.schemes, vec!["bearer"]);
        assert_eq!(card.skills.len(), 1);
        assert_eq!(card.skills[0].id, "translate");
        assert_eq!(card.skills[0].input_modes, vec!["text"]);
        assert!(card.capabilities.streaming);
        assert!(!card.capabilities.push_notifications);
    }

    #[test]
    fn accepts_snake_case_modes_alias() {
        let json = r#"{
            "name": "x", "description": "", "url": "https://x.test", "version": "1",
            "skills": [{
                "id": "s", "name": "S",
                "input_modes": ["text"], "output_modes": ["text"]
            }]
        }"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.skills[0].input_modes, vec!["text"]);
    }

    #[test]
    fn preserves_unknown_top_level_fields() {
        let json = r#"{
            "name": "x", "description": "", "url": "https://x.test", "version": "1",
            "futureFlag": true
        }"#;
        let card: AgentCard = serde_json::from_str(json).unwrap();
        assert!(card.extra.contains_key("futureFlag"));
    }

    #[test]
    fn round_trip_serialization() {
        let card: AgentCard = serde_json::from_str(FIXTURE).unwrap();
        let s = serde_json::to_string(&card).unwrap();
        let card2: AgentCard = serde_json::from_str(&s).unwrap();
        assert_eq!(card.name, card2.name);
        assert_eq!(card.skills.len(), card2.skills.len());
    }
}
