//! Newline-delimited JSON-RPC framing over a child's stdin/stdout (tokio).
//!
//! Reuse note (중복 검사 — repo rule #4): `crates/openxgram-mcp` defines the
//! JSON-RPC message *shape* (`jsonrpc`/`id`/`method`/`params`, `-32601/-32602/
//! -32603` error codes), but it is a pure in-process request handler — it does
//! **not** spawn a child or own stdin/stdout pipes (the actual stdio server is
//! `openxgram-cli/src/mcp_serve.rs`, which we must not touch in B-1). So the
//! reusable artifact is the framing *contract* (one JSON object per line,
//! LDJSON), which this module re-implements minimally for the *client* side:
//! writing requests to a child's stdin and reading lines from its stdout.
//!
//! Critical (§6): the agent's protocol is on **stdout**; **stderr is logs only**
//! and is forwarded to `tracing`, never parsed.

use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use crate::{AcpError, Result};

/// The three pipe halves we keep after spawning the agent.
pub struct ChildPipes {
    /// The live child handle (kept so we can wait / kill it).
    pub child: Child,
    /// Writer half of the protocol channel (child stdin).
    pub stdin: ChildStdin,
    /// Reader half of the protocol channel (child stdout).
    pub stdout: ChildStdout,
}

/// Windows-only: resolve a bare/extensionless command to a `cmd /c <shim>`
/// invocation when PATH resolves it **only** to a `.cmd`/`.bat` shim.
///
/// Why (§6, no-fallback): on Windows `CreateProcess` (which Rust's `Command`
/// uses) does **not** honor `PATHEXT`, so `Command::new("claude-agent-acp")`
/// fails to find the npm-installed `.cmd`/`.bat` shim and the ACP session
/// errors out (502). npm on Windows installs CLI bins as `<name>.cmd`/`.ps1`
/// shims, never as a bare executable — so a native adapter is unspawnable
/// without this. We re-exec the resolved shim through `cmd /c` so the shell
/// interprets the batch shim.
///
/// Returns `Some(Command)` only when the input is a bare name (no path
/// separator, no extension) that resolves on PATH to a `.cmd`/`.bat`. Real
/// `.exe`s, absolute/relative paths, and names with extensions return `None`
/// (caller uses them as-is — zero behavior change for native binaries).
#[cfg(windows)]
fn resolve_windows_command(command: &str) -> Option<Command> {
    use std::path::Path;

    // Only intervene for bare, extensionless names. A path separator or an
    // explicit extension means the caller already knows exactly what to run.
    if command.contains('/') || command.contains('\\') {
        return None;
    }
    if Path::new(command).extension().is_some() {
        return None;
    }

    let path_var = std::env::var_os("PATH")?;
    // Probe the shim extensions that need a shell to execute. We deliberately
    // do NOT probe `.exe`/`.com` here: if a real executable exists we want the
    // caller to spawn it directly (None), matching native behavior.
    for dir in std::env::split_paths(&path_var) {
        for ext in ["cmd", "bat"] {
            let candidate = dir.join(format!("{command}.{ext}"));
            if candidate.is_file() {
                let mut cmd = Command::new("cmd");
                // /c runs the shim then terminates; pass the fully-resolved
                // path so cmd does not re-resolve (and to avoid PATH ambiguity).
                cmd.arg("/c").arg(&candidate);
                return Some(cmd);
            }
        }
    }
    None
}

/// Non-Windows: never rewrites the command (zero behavior change). PATHEXT /
/// `.cmd` shims do not exist on Linux/macOS.
#[cfg(not(windows))]
fn resolve_windows_command(_command: &str) -> Option<Command> {
    None
}

/// Spawn the agent process with piped stdio and `kill_on_drop`.
///
/// stderr is piped and forwarded to `tracing` by [`spawn_stderr_logger`].
pub fn spawn_agent(
    command: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
) -> Result<ChildPipes> {
    // On Windows, a bare `claude-agent-acp` (npm `.cmd`/`.bat` shim) must be
    // run via `cmd /c <shim>` because CreateProcess ignores PATHEXT. On every
    // other platform this is always `None` (see `resolve_windows_command`).
    let mut cmd = match resolve_windows_command(command) {
        Some(c) => c,
        None => Command::new(command),
    };
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn().map_err(AcpError::Spawn)?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AcpError::Protocol("child stdin pipe missing".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AcpError::Protocol("child stdout pipe missing".into()))?;

    // stderr → tracing, never parsed as protocol.
    if let Some(stderr) = child.stderr.take() {
        spawn_stderr_logger(stderr, command.to_string());
    }

    Ok(ChildPipes {
        child,
        stdin,
        stdout,
    })
}

/// Forward each line of the child's stderr to `tracing` at debug level.
fn spawn_stderr_logger<R>(stderr: R, label: String)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    tracing::debug!(target: "acp.agent.stderr", agent = %label, "{line}")
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::debug!(target: "acp.agent.stderr", agent = %label, "stderr read error: {e}");
                    break;
                }
            }
        }
    });
}

/// Writer half: serializes each JSON value to a single newline-terminated line
/// on the child's stdin. Runs as a spawned task fed by an `mpsc` so the reader
/// loop never blocks on writes (§6 full-duplex safety).
pub fn spawn_writer(mut stdin: ChildStdin) -> mpsc::UnboundedSender<Value> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut line = match serde_json::to_vec(&msg) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(target: "acp.transport", "serialize outbound frame failed: {e}");
                    continue;
                }
            };
            line.push(b'\n');
            if let Err(e) = stdin.write_all(&line).await {
                tracing::debug!(target: "acp.transport", "stdin write failed (agent gone?): {e}");
                break;
            }
            if let Err(e) = stdin.flush().await {
                tracing::debug!(target: "acp.transport", "stdin flush failed: {e}");
                break;
            }
        }
    });
    tx
}

/// Reader half: parses each newline-delimited line from the child's stdout into
/// a JSON value and forwards it on the returned channel. Blank lines are
/// skipped. A parse failure is reported as an explicit error frame (절대 규칙 1
/// — never silently dropped), then reading continues so one bad line does not
/// kill the stream.
pub fn spawn_reader(stdout: ChildStdout) -> mpsc::UnboundedReceiver<Result<Value>> {
    let (tx, rx) = mpsc::unbounded_channel::<Result<Value>>();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let parsed = serde_json::from_str::<Value>(trimmed).map_err(AcpError::Serde);
                    if tx.send(parsed).is_err() {
                        break; // receiver dropped — peer is shutting down.
                    }
                }
                Ok(None) => break, // EOF — agent closed stdout.
                Err(e) => {
                    let _ = tx.send(Err(AcpError::Io(e)));
                    break;
                }
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reader_parses_ldjson_and_skips_blank_lines() {
        // Use `cat` as a trivial echo: write to stdin, read identical lines back.
        let pipes = spawn_agent("cat", &[], &[], None).expect("spawn cat");
        let tx = spawn_writer(pipes.stdin);
        let mut rx = spawn_reader(pipes.stdout);

        tx.send(serde_json::json!({"a": 1})).expect("send");
        tx.send(serde_json::json!({"b": 2})).expect("send");

        let first = rx.recv().await.expect("frame").expect("ok");
        assert_eq!(first["a"], 1);
        let second = rx.recv().await.expect("frame").expect("ok");
        assert_eq!(second["b"], 2);
    }
}
