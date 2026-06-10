//! End-to-end mock-agent test (research §2.2 deliverable).
//!
//! Spawns a tiny Python "agent" that speaks newline-delimited JSON-RPC 2.0 on
//! stdin/stdout: it answers `initialize`, `session/new`, and `session/prompt`
//! (emitting one `session/update` agent_message_chunk, then resolving the
//! prompt with `stopReason: end_turn`). This exercises the full path:
//! spawn → initialize → session/new → prompt → stopReason.

use std::io::Write;

use openxgram_acp::registry::AgentSpec;
use openxgram_acp::types::{ContentBlock, StopReason};
use openxgram_acp::AcpClient;

/// A self-contained mock ACP agent. Reads one JSON object per line, writes one
/// JSON object per line. stderr is used for nothing protocol-relevant.
const MOCK_AGENT_PY: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception as e:
        sys.stderr.write("parse error: %s\n" % e)
        continue
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": False,
                    "promptCapabilities": {"image": False, "audio": False, "embeddedContext": False}
                },
                "agentInfo": {"name": "mock-agent", "version": "0.0.1"},
                "authMethods": []
            }
        })
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": "mock-sess-1"}})
    elif method == "session/prompt":
        params = msg.get("params", {})
        sid = params.get("sessionId", "mock-sess-1")
        # Stream session/update notifications using the REAL ACP wire shape:
        # the discriminator + payload are nested under params.update, alongside
        # params.sessionId (matches @agentclientprotocol/sdk SessionNotification).
        def update(body):
            send({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {"sessionId": sid, "update": body},
            })
        # 1) a thought chunk, 2) two message chunks, in order.
        update({"sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "thinking"}})
        update({"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "hello from mock"}})
        update({"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": " (part two)"}})
        # A future/unmodeled update kind — must be surfaced as Unknown, not dropped.
        update({"sessionUpdate": "some_future_update_kind", "blob": {"x": 1}})
        # Resolve the turn.
        send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
    elif method == "session/cancel":
        # Notification; nothing to reply. A real agent would resolve the
        # in-flight prompt with cancelled.
        pass
    else:
        if mid is not None:
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "method not found"}})
"#;

fn write_mock_agent() -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .prefix("mock-acp-agent-")
        .suffix(".py")
        .tempfile()
        .expect("create temp mock agent");
    f.write_all(MOCK_AGENT_PY.as_bytes())
        .expect("write mock agent");
    f.flush().expect("flush mock agent");
    f
}

fn python_command() -> &'static str {
    // Prefer python3; the CI image has it.
    "python3"
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_agent_full_turn() {
    let mock = write_mock_agent();
    let path = mock.path().to_string_lossy().to_string();

    let spec = AgentSpec::builder("mock", python_command())
        .arg(path)
        .build();

    let client = AcpClient::spawn_minimal(spec)
        .await
        .expect("spawn + initialize mock agent");

    // initialize negotiated capabilities are visible.
    assert!(!client.agent_capabilities().load_session);

    // session/new + session/prompt in one shot.
    let result = client
        .prompt("/tmp", vec![ContentBlock::text("hi")])
        .await
        .expect("prompt turn");

    assert_eq!(result.stop_reason, StopReason::EndTurn);

    // REGRESSION (streaming gap): the agent streamed four `session/update`
    // notifications DURING the turn (before the stopReason). They use the real
    // wire shape (`params.update.sessionUpdate`), so they MUST be collected and
    // returned — not lost to a deser/shape mismatch (the rc.294 empty-updates bug).
    use openxgram_acp::types::SessionUpdate;
    assert_eq!(
        result.updates.len(),
        4,
        "expected all 4 streamed session/update notifications, got {}: {:?}",
        result.updates.len(),
        result.updates
    );

    // ...and IN ORDER: thought, message, message, unknown(usage_update).
    let texts: Vec<&str> = result
        .updates
        .iter()
        .filter_map(|u| match u {
            SessionUpdate::AgentThoughtChunk {
                content: ContentBlock::Text { text },
            }
            | SessionUpdate::AgentMessageChunk {
                content: ContentBlock::Text { text },
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec!["thinking", "hello from mock", " (part two)"],
        "streamed chunks must be collected in arrival order"
    );

    assert!(
        matches!(result.updates[0], SessionUpdate::AgentThoughtChunk { .. }),
        "first update should be the thought chunk"
    );

    // The unmodeled `usage_update` must be surfaced as Unknown (not dropped).
    assert!(
        matches!(result.updates[3], SessionUpdate::Unknown),
        "unknown update kind must be surfaced as Unknown, got {:?}",
        result.updates[3]
    );

    client.close().await.expect("close + reap");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_nonexistent_binary_is_spawn_error() {
    let spec = AgentSpec::builder("nope", "this-binary-does-not-exist-xyz").build();
    match AcpClient::spawn_minimal(spec).await {
        Err(openxgram_acp::AcpError::Spawn(_)) => {}
        Err(other) => panic!("expected Spawn error, got {other:?}"),
        Ok(_) => panic!("expected spawn to fail for nonexistent binary"),
    }
}
