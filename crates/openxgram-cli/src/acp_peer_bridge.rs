//! ACP ↔ peer auto-bridge — pure mapping + decision helpers (rc.355).
//!
//! Today ACP sessions (`/v1/acp/*`, daemon-spawned agent sessions) live in an
//! isolated plane (`AcpHttpState.sessions` in-memory + `acp_messages` table).
//! They are **not** registered as peers, so they don't appear in the roster /
//! `list_peers` and can't be a `peer_send` / A2A target.
//!
//! This module owns the **pure** (DB-free, side-effect-free) logic that the
//! bridge needs — kept here so it is unit-testable without a daemon:
//!   - [`acp_session_identifier`] — the canonical `acp:<sessionId>` marker we
//!     stamp onto a bridged peer row (`peers.session_identifier`).
//!   - [`is_acp_backed`] — the routing decision: does a peer's
//!     `session_identifier` mean "deliver by driving the ACP prompt" instead of
//!     signing + enqueuing a transport envelope?
//!   - [`PeerUpsert`] + [`map_session_to_peer_upsert`] — the value object the
//!     spawn path feeds into the existing `agent_capabilities` / `agent_profiles`
//!     UPSERT (no new schema; reuses the `register_subagent` rows the roster
//!     reads via `IdentityStore::roster`).
//!
//! 절대 규칙 1 (fallback 금지): callers log/propagate every DB error; these pure
//! helpers never swallow anything — they only transform inputs.

/// `peers.session_identifier` scheme prefix for an ACP-backed bridged peer.
/// Mirrors the existing `tmux:<name>` convention used by terminal peers.
pub const ACP_SID_PREFIX: &str = "acp:";

/// ack_status set on the sender's `outbound_queue` row when a `peer_send` was
/// delivered by driving the target's ACP `session/prompt` (rather than the
/// transport `inbox_stored` / `tmux_injected` paths).
pub const ACK_ACP_PROMPTED: &str = "acp_prompted";

/// Build the canonical `acp:<sessionId>` marker stored in
/// `peers.session_identifier` for a bridged ACP session.
pub fn acp_session_identifier(session_id: &str) -> String {
    format!("{ACP_SID_PREFIX}{session_id}")
}

/// Routing decision: is this peer backed by a live ACP session (so delivery is
/// "drive the prompt") rather than a transport peer ("sign + enqueue")?
///
/// `session_identifier` is the `peers.session_identifier` column (may be `None`).
pub fn is_acp_backed(session_identifier: Option<&str>) -> bool {
    session_identifier
        .map(|s| s.starts_with(ACP_SID_PREFIX))
        .unwrap_or(false)
}

/// Extract the ACP session id from an `acp:<sessionId>` marker. Returns `None`
/// if the marker is not ACP-backed (so callers can fall back cleanly).
pub fn acp_session_id_of(session_identifier: Option<&str>) -> Option<String> {
    session_identifier
        .filter(|s| s.starts_with(ACP_SID_PREFIX))
        .map(|s| s[ACP_SID_PREFIX.len()..].to_string())
}

/// Value object: everything the spawn path needs to UPSERT a bridged ACP
/// session into the roster + routing tables. Produced purely from the session
/// snapshot so it can be unit-tested; the DB write lives in the daemon caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerUpsert {
    /// Roster + routing key (= the ACP session `label` = agent identity / alias).
    pub alias: String,
    /// `peers.session_identifier` value (`acp:<sessionId>`).
    pub session_identifier: String,
    /// `agent_profiles.ai_type` — derived from the ACP adapter name so the
    /// existing A2A `new_acp` resolution picks the right adapter on delivery.
    pub ai_type: String,
    /// `agent_capabilities.role` — non-`tmux` so `is_acp_drivable` returns true.
    pub role: String,
}

/// Map an ACP adapter/agent name to the `agent_profiles.ai_type` enum the rest
/// of the daemon understands (`claude` | `codex` | `gemini`). Unknown adapters
/// default to `claude` (the A2A resolver's own default) — never an error here,
/// because an unmapped adapter must still be reachable, not silently dropped.
pub fn ai_type_for_agent(agent: &str) -> &'static str {
    match agent {
        a if a.contains("codex") => "codex",
        a if a.contains("gemini") => "gemini",
        _ => "claude",
    }
}

/// Pure mapping: ACP session snapshot → [`PeerUpsert`]. Returns `None` when the
/// session has no usable identity (empty `label`) — a picker-entry session that
/// must **not** be bridged (matches the `label`-keyed reuse rule in
/// `daemon_gui_acp::create_session`).
pub fn map_session_to_peer_upsert(
    session_id: &str,
    label: Option<&str>,
    agent: &str,
) -> Option<PeerUpsert> {
    let alias = label.map(str::trim).filter(|s| !s.is_empty())?;
    Some(PeerUpsert {
        alias: alias.to_string(),
        session_identifier: acp_session_identifier(session_id),
        ai_type: ai_type_for_agent(agent).to_string(),
        // ACP-backed agents get a generic non-tmux role so `is_acp_drivable`
        // (role IS NOT 'tmux') treats them as drivable. Existing rows keep their
        // own role via the caller's COALESCE/ON CONFLICT — this is the seed.
        role: "acp".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_session_identifier_prefixes() {
        assert_eq!(acp_session_identifier("acp-7"), "acp:acp-7");
        assert_eq!(acp_session_identifier("abc"), "acp:abc");
    }

    #[test]
    fn is_acp_backed_matches_only_acp_prefix() {
        assert!(is_acp_backed(Some("acp:acp-7")));
        assert!(is_acp_backed(Some("acp:anything")));
        assert!(!is_acp_backed(Some("tmux:my-session")));
        assert!(!is_acp_backed(Some("")));
        assert!(!is_acp_backed(None));
    }

    #[test]
    fn acp_session_id_of_extracts_or_none() {
        assert_eq!(acp_session_id_of(Some("acp:acp-7")).as_deref(), Some("acp-7"));
        assert_eq!(acp_session_id_of(Some("tmux:x")), None);
        assert_eq!(acp_session_id_of(None), None);
    }

    #[test]
    fn ai_type_maps_adapter_names() {
        assert_eq!(ai_type_for_agent("claude-agent-acp"), "claude");
        assert_eq!(ai_type_for_agent("codex-acp"), "codex");
        assert_eq!(ai_type_for_agent("gemini-acp"), "gemini");
        assert_eq!(ai_type_for_agent("unknown-adapter"), "claude");
    }

    #[test]
    fn map_session_requires_nonempty_label() {
        // picker entry (no label) → not bridged.
        assert_eq!(map_session_to_peer_upsert("acp-1", None, "claude-agent-acp"), None);
        assert_eq!(map_session_to_peer_upsert("acp-1", Some("   "), "claude-agent-acp"), None);
    }

    #[test]
    fn map_session_builds_upsert_for_labeled_session() {
        let got = map_session_to_peer_upsert("acp-9", Some("Eno"), "codex-acp")
            .expect("labeled session bridges");
        assert_eq!(
            got,
            PeerUpsert {
                alias: "Eno".to_string(),
                session_identifier: "acp:acp-9".to_string(),
                ai_type: "codex".to_string(),
                role: "acp".to_string(),
            }
        );
        // the routing decision agrees with what we stamped.
        assert!(is_acp_backed(Some(&got.session_identifier)));
        assert_eq!(
            acp_session_id_of(Some(&got.session_identifier)).as_deref(),
            Some("acp-9")
        );
    }

    #[test]
    fn map_session_trims_label() {
        let got = map_session_to_peer_upsert("acp-2", Some("  Pip  "), "claude-agent-acp")
            .expect("trimmed label bridges");
        assert_eq!(got.alias, "Pip");
    }
}
