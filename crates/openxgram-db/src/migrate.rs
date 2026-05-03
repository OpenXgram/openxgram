use crate::error::DbError;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone)]
pub struct MigrationRecord {
    pub version: u32,
    pub name: String,
    pub applied_at: String,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "message_embeddings",
        sql: include_str!("../migrations/0002_message_embeddings.sql"),
    },
    Migration {
        version: 3,
        name: "episodes",
        sql: include_str!("../migrations/0003_episodes.sql"),
    },
    Migration {
        version: 4,
        name: "patterns",
        sql: include_str!("../migrations/0004_patterns.sql"),
    },
    Migration {
        version: 5,
        name: "traits",
        sql: include_str!("../migrations/0005_traits.sql"),
    },
    Migration {
        version: 6,
        name: "vault",
        sql: include_str!("../migrations/0006_vault.sql"),
    },
    Migration {
        version: 7,
        name: "vault_acl",
        sql: include_str!("../migrations/0007_vault_acl.sql"),
    },
    Migration {
        version: 8,
        name: "vault_pending",
        sql: include_str!("../migrations/0008_vault_pending.sql"),
    },
    Migration {
        version: 9,
        name: "mcp_tokens",
        sql: include_str!("../migrations/0009_mcp_tokens.sql"),
    },
    Migration {
        version: 10,
        name: "peers",
        sql: include_str!("../migrations/0010_peers.sql"),
    },
];

pub struct MigrationRunner<'a> {
    conn: &'a mut rusqlite::Connection,
}

impl<'a> MigrationRunner<'a> {
    pub fn new(conn: &'a mut rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub fn run_all(&mut self) -> Result<(), DbError> {
        // schema_migrations 테이블 보장
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )",
            [],
        )?;

        for m in MIGRATIONS {
            let already: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [m.version],
                |r| r.get(0),
            )?;
            if already {
                tracing::debug!(
                    version = m.version,
                    name = m.name,
                    "migration already applied, skipping"
                );
                continue;
            }

            tracing::info!(version = m.version, name = m.name, "applying migration");

            let tx = self.conn.transaction()?;
            tx.execute_batch(m.sql).map_err(|e| DbError::Migration {
                version: m.version,
                reason: e.to_string(),
            })?;

            let now = chrono::Local::now().to_rfc3339();
            let affected = tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![m.version, m.name, now],
            )?;

            // silent error 방지 — affected_rows 검증
            if affected != 1 {
                return Err(DbError::UnexpectedRowCount {
                    expected: 1,
                    actual: affected as u64,
                });
            }
            tx.commit()?;
        }
        Ok(())
    }
}
