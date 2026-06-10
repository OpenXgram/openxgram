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

/// LIVE STREAMING (per-chunk): the `on_update` channel must receive each
/// `session/update` body **as it arrives during the turn** — i.e. before the
/// prompt future resolves with the `stopReason`. This proves the updates are
/// forwarded live, not collected-then-dumped at turn end.
///
/// We assert live ordering by polling the on_update receiver while the prompt
/// future is still pending: at least the first update must be observable before
/// the turn completes, and every update body is delivered (count + order).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_streaming_forwards_updates_live() {
    use std::time::Duration;
    use tokio::sync::mpsc;

    let mock = write_mock_agent();
    let path = mock.path().to_string_lossy().to_string();
    let spec = AgentSpec::builder("mock", python_command())
        .arg(path)
        .build();

    let client = AcpClient::spawn_minimal(spec)
        .await
        .expect("spawn + initialize mock agent");

    let (tx, mut rx) = mpsc::unbounded_channel::<serde_json::Value>();

    // Drive the turn on a separate task so we can observe live forwarding while
    // the prompt future is still pending.
    let turn = tokio::spawn(async move {
        client
            .prompt_streaming("/tmp", vec![ContentBlock::text("hi")], Some(tx))
            .await
    });

    // LIVE ORDERING: the first update must arrive on the channel before the turn
    // future resolves. `recv()` here returns the live-forwarded body; if updates
    // were only collected-then-returned, the channel would stay empty until the
    // turn ended (and then close), so an early `recv()` proves liveness.
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("first live update should arrive before turn end (liveness)")
        .expect("channel open mid-turn — sender not yet dropped");
    assert!(!turn.is_finished(), "turn still pending when first update seen");
    assert_eq!(
        first.get("sessionUpdate").and_then(|v| v.as_str()),
        Some("agent_thought_chunk"),
        "first live update body must be the thought chunk (got {first})"
    );

    // Collect the rest of the live stream until the sender drops (turn ends).
    let mut bodies = vec![first];
    while let Some(b) = rx.recv().await {
        bodies.push(b);
    }

    // The turn future resolves with the full PromptResult; the live stream
    // delivered the SAME set of updates (additive — return contract unchanged).
    let result = turn
        .await
        .expect("turn task join")
        .expect("prompt turn ok");
    assert_eq!(result.stop_reason, StopReason::EndTurn);
    assert_eq!(
        bodies.len(),
        result.updates.len(),
        "live channel must deliver every update the collected vec has"
    );
    assert_eq!(bodies.len(), 4, "all 4 updates forwarded live");

    let disc: Vec<&str> = bodies
        .iter()
        .filter_map(|b| b.get("sessionUpdate").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        disc,
        vec![
            "agent_thought_chunk",
            "agent_message_chunk",
            "agent_message_chunk",
            "some_future_update_kind",
        ],
        "live updates must arrive in order"
    );
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
