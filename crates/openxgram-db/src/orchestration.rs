//! Paperclip orchestration absorption — Phase 1 core-entity models.
//!
//! Typed mirrors of the tables added in migration 0047. Queries themselves live in the
//! GUI/MCP layer (daemon_gui.rs uses serde_json rows, following existing convention); these
//! structs are the canonical column contract for serialization and future typed access.
//!
//! Source spec: docs/research/paperclip-orchestration-extraction.md §1.

use serde::{Deserialize, Serialize};

/// org container (tenant). Company-scoped isolation mirrors the maker_id rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub pause_reason: Option<String>,
    pub paused_at: Option<String>,
    pub issue_prefix: Option<String>,
    pub issue_counter: i64,
    pub budget_monthly_cents: Option<i64>,
    pub spent_monthly_cents: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Org-chart node overlay columns added to `agent_capabilities`.
/// (The base profile columns live on the existing agent_capabilities row.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOrchestrationOverlay {
    pub alias: String,
    pub company_id: Option<String>,
    pub reports_to: Option<String>,
    pub adapter_type: Option<String>,
    pub adapter_config: Option<String>,
    pub budget_monthly_cents: Option<i64>,
    pub status: Option<String>,
    pub paused_at: Option<String>,
}

/// Goal with self-referential `parent_id` ancestry tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub company_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub level: String,
    pub status: String,
    pub parent_id: Option<String>,
    pub owner_agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub company_id: Option<String>,
    pub goal_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub lead_agent_id: Option<String>,
    pub env: Option<String>,
    pub target_date: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// M:N join between projects and goals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGoal {
    pub project_id: String,
    pub goal_id: String,
}

/// Unit of work. `parent_id` = issue tree / child fan-out;
/// `checkout_run_id` + `execution_locked_at` = atomic checkout lock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub company_id: Option<String>,
    pub project_id: Option<String>,
    pub goal_id: Option<String>,
    pub parent_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub status: String,
    pub priority: i64,
    pub assignee_agent_id: Option<String>,
    pub checkout_run_id: Option<String>,
    pub execution_locked_at: Option<String>,
    pub origin_kind: Option<String>,
    pub origin_fingerprint: Option<String>,
    pub request_depth: i64,
    pub issue_number: Option<i64>,
    pub identifier: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Dependency edge between issues (DAG); `relation_type` default "blocks".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRelation {
    pub id: String,
    pub company_id: Option<String>,
    pub issue_id: String,
    pub related_issue_id: String,
    pub relation_type: String,
    pub created_at: String,
}

/// Unified event feed row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityLog {
    pub id: String,
    pub company_id: Option<String>,
    pub actor: Option<String>,
    pub kind: String,
    pub target: Option<String>,
    pub payload: Option<String>,
    pub created_at: String,
}
