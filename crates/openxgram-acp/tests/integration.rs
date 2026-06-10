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
        # Stream one agent_message_chunk update (notification, no id).
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "hello from mock"}
            }
        })
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
    assert!(
        !result.updates.is_empty(),
        "expected at least one session/update"
    );

    // The streamed chunk should carry our mock text.
    let has_chunk = result.updates.iter().any(|u| {
        matches!(
            u,
            openxgram_acp::types::SessionUpdate::AgentMessageChunk { content }
            if matches!(content, ContentBlock::Text { text } if text == "hello from mock")
        )
    });
    assert!(has_chunk, "expected agent_message_chunk with mock text");

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
