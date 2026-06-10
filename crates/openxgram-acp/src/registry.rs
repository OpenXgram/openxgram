//! Known ACP agent adapters: name → spawn spec (§2.2, §6 references).
//!
//! Maps a friendly agent name to the command/args/env needed to launch it as
//! an ACP subprocess. New agents are added here as the registry grows
//! (claude-agent-acp, codex-acp, gemini --acp, opencode acp, ...).
//!
//! 절대 규칙 1: an unknown name returns [`crate::AcpError::UnknownAgent`], never
//! a guessed default.

use crate::{AcpError, Result};

/// How to spawn one ACP agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    /// Friendly registry name (e.g. `claude-agent-acp`).
    pub name: String,
    /// Executable to run.
    pub command: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Extra environment variables (key, value).
    pub env: Vec<(String, String)>,
}

impl AgentSpec {
    /// Start building a spec for a custom agent.
    pub fn builder(name: impl Into<String>, command: impl Into<String>) -> AgentSpecBuilder {
        AgentSpecBuilder {
            spec: AgentSpec {
                name: name.into(),
                command: command.into(),
                args: Vec::new(),
                env: Vec::new(),
            },
        }
    }
}

/// Fluent builder for [`AgentSpec`].
#[derive(Debug, Clone)]
pub struct AgentSpecBuilder {
    spec: AgentSpec,
}

impl AgentSpecBuilder {
    /// Append a single argument.
    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.spec.args.push(a.into());
        self
    }

    /// Append many arguments.
    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.spec.args.extend(it.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable.
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.spec.env.push((k.into(), v.into()));
        self
    }

    /// Finish building.
    pub fn build(self) -> AgentSpec {
        self.spec
    }
}

/// The list of known, built-in ACP agent names.
pub const KNOWN_AGENTS: &[&str] = &["claude-agent-acp", "codex-acp", "gemini", "opencode"];

/// Look up a built-in agent spec by name.
///
/// These mirror the adapters listed in research §2.2 / §6. The exact npm
/// package install (`npx`) is left to deployment; here we encode the canonical
/// invocation shape.
pub fn lookup(name: &str) -> Result<AgentSpec> {
    let spec = match name {
        "claude-agent-acp" => AgentSpec::builder("claude-agent-acp", "claude-agent-acp").build(),
        "codex-acp" => AgentSpec::builder("codex-acp", "codex-acp").build(),
        "gemini" => AgentSpec::builder("gemini", "gemini").arg("--acp").build(),
        "opencode" => AgentSpec::builder("opencode", "opencode")
            .arg("acp")
            .build(),
        other => return Err(AcpError::UnknownAgent(other.to_string())),
    };
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_agents() {
        assert_eq!(
            lookup("claude-agent-acp").unwrap().command,
            "claude-agent-acp"
        );
        assert_eq!(lookup("gemini").unwrap().args, vec!["--acp".to_string()]);
        assert_eq!(lookup("opencode").unwrap().args, vec!["acp".to_string()]);
    }

    #[test]
    fn lookup_unknown_is_explicit_error() {
        let err = lookup("nope-acp").unwrap_err();
        matches!(err, AcpError::UnknownAgent(_));
    }

    #[test]
    fn builder_assembles_spec() {
        let s = AgentSpec::builder("x", "node")
            .arg("server.js")
            .env("KEY", "v")
            .build();
        assert_eq!(s.command, "node");
        assert_eq!(s.args, vec!["server.js".to_string()]);
        assert_eq!(s.env, vec![("KEY".to_string(), "v".to_string())]);
    }
}
