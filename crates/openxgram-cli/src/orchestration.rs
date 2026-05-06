//! CLI 바인딩 — `xgram schedule ...` 와 `xgram chain ...`.
//!
//! 다른 에이전트 (channel-mcp-embed) 가 추가하는 RouteEngine 가
//! [`openxgram_orchestration::ChannelSender`] 를 구현하면 자동 연동.
//! 머지 전엔 `NoopSender` 로 dry-run.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    chain::ChainDefinition, kst_now_epoch, ChainRunner, ChainStore, NoopSender, ScheduleKind,
    ScheduledStatus, ScheduledStore, TargetKind,
};

#[derive(Debug, Clone, clap::Subcommand)]
pub enum ScheduleAction {
    /// 1회성 예약 (특정 시각).
    /// `--at` 은 ISO8601 형식 (`2026-05-05T09:00:00+09:00` 또는 KST naive `2026-05-05 09:00:00`).
    Once {
        #[arg(long)]
        at: String,
        #[arg(long, conflicts_with_all = ["to_platform", "target_kind"])]
        to_role: Option<String>,
        #[arg(long, conflicts_with_all = ["to_role", "target_kind"])]
        to_platform: Option<String>,
        #[arg(long, requires = "to_platform")]
        channel_id: Option<String>,
        /// `role` | `platform` — `--target` 와 함께 라우팅. 기본은 role:master.
        #[arg(long, value_parser = ["role", "platform"])]
        target_kind: Option<String>,
        /// role 일 때는 role 이름 (예: master, eno), platform 일 때는 `discord:CHID`.
        #[arg(long, requires = "target_kind")]
        target: Option<String>,
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "info")]
        msg_type: String,
    },
    /// 반복 예약 (cron 표현식, KST 기준).
    Cron {
        /// cron expr (5 또는 6 필드). 예: `"0 9 * * *"` (매일 09:00 KST).
        cron_expr: String,
        #[arg(long, conflicts_with_all = ["to_platform", "target_kind"])]
        to_role: Option<String>,
        #[arg(long, conflicts_with_all = ["to_role", "target_kind"])]
        to_platform: Option<String>,
        #[arg(long, requires = "to_platform")]
        channel_id: Option<String>,
        #[arg(long, value_parser = ["role", "platform"])]
        target_kind: Option<String>,
        #[arg(long, requires = "target_kind")]
        target: Option<String>,
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "info")]
        msg_type: String,
    },
    /// 예약 목록.
    List {
        /// `pending` | `sent` | `failed` | `cancelled`.
        #[arg(long)]
        status: Option<String>,
    },
    /// 취소.
    Cancel { id: String },
    /// 도달한 메시지 수동 실행 (테스트용 — daemon 없이 dry-run).
    RunPending,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum ChainAction {
    /// YAML 정의 파일로 체인 등록.
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    /// 등록된 체인 목록.
    List,
    /// 체인 상세 보기 (steps).
    Show { name: String },
    /// 체인 실행 (NoopSender — channel embed 머지 후 실제 송신 연결).
    Run { name: String },
    /// 체인 삭제.
    Delete { name: String },
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

fn resolve_target(
    to_role: Option<String>,
    to_platform: Option<String>,
    channel_id: Option<String>,
    target_kind: Option<String>,
    target: Option<String>,
) -> Result<(TargetKind, String)> {
    // 신 시그니처 — `--target-kind role|platform` + `--target ...`
    if let Some(kind) = target_kind {
        let value = target.ok_or_else(|| anyhow!("--target required with --target-kind"))?;
        return match kind.as_str() {
            "role" => Ok((TargetKind::Role, value)),
            "platform" => Ok((TargetKind::Platform, value)),
            other => Err(anyhow!(
                "--target-kind must be `role` or `platform` (got `{other}`)"
            )),
        };
    }
    // 기존 시그니처 (호환).
    if let Some(role) = to_role {
        return Ok((TargetKind::Role, role));
    }
    if let Some(platform) = to_platform {
        let ch = channel_id.ok_or_else(|| anyhow!("--channel-id required with --to-platform"))?;
        return Ok((TargetKind::Platform, format!("{platform}:{ch}")));
    }
    // 기본값: role:master — 사이트 안내와 일치.
    Ok((TargetKind::Role, "master".to_string()))
}

pub fn run_schedule(data_dir: &Path, action: ScheduleAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let store = ScheduledStore::new(db.conn());
    match action {
        ScheduleAction::Once {
            at,
            to_role,
            to_platform,
            channel_id,
            target_kind,
            target,
            text,
            msg_type,
        } => {
            let (kind, target) =
                resolve_target(to_role, to_platform, channel_id, target_kind, target)?;
            let id = store
                .insert(kind, &target, &text, &msg_type, ScheduleKind::Once, &at)
                .context("INSERT scheduled (once)")?;
            println!(
                "scheduled once: {id}  at={at}  → {}:{target}",
                kind.as_str()
            );
            Ok(())
        }
        ScheduleAction::Cron {
            cron_expr,
            to_role,
            to_platform,
            channel_id,
            target_kind,
            target,
            text,
            msg_type,
        } => {
            let (kind, target) =
                resolve_target(to_role, to_platform, channel_id, target_kind, target)?;
            let id = store
                .insert(
                    kind,
                    &target,
                    &text,
                    &msg_type,
                    ScheduleKind::Cron,
                    &cron_expr,
                )
                .context("INSERT scheduled (cron)")?;
            println!(
                "scheduled cron: {id}  expr=`{cron_expr}` (KST)  → {}:{target}",
                kind.as_str()
            );
            Ok(())
        }
        ScheduleAction::List { status } => {
            let filter = status
                .as_deref()
                .map(|s| s.parse::<ScheduledStatus>())
                .transpose()
                .map_err(|e| anyhow!("invalid status: {e}"))?;
            let rows = store.list(filter).context("SELECT scheduled")?;
            if rows.is_empty() {
                println!("(no scheduled messages)");
                return Ok(());
            }
            for m in rows {
                println!(
                    "- {id} [{status}] {kind}={target} schedule={skind}:`{sval}` next_due={next:?}",
                    id = m.id,
                    status = m.status.as_str(),
                    kind = m.target_kind.as_str(),
                    target = m.target,
                    skind = m.schedule_kind.as_str(),
                    sval = m.schedule_value,
                    next = m.next_due_at_kst,
                );
                if let Some(err) = &m.last_error {
                    println!("    last_error: {err}");
                }
            }
            Ok(())
        }
        ScheduleAction::Cancel { id } => {
            store.cancel(&id).context("UPDATE scheduled (cancel)")?;
            println!("cancelled: {id}");
            Ok(())
        }
        ScheduleAction::RunPending => {
            let now = kst_now_epoch();
            let due = store.list_due(now).context("list_due")?;
            println!("due: {} message(s)", due.len());
            for m in due {
                // channel embed 미머지 — NoopSender 로 dry-run 표시만
                println!(
                    "  would send → {}:{} payload=`{}`",
                    m.target_kind.as_str(),
                    m.target,
                    m.payload
                );
                store.mark_sent(&m.id).context("mark_sent (dry-run)")?;
            }
            Ok(())
        }
    }
}

pub fn run_chain(data_dir: &Path, action: ChainAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let store = ChainStore::new(db.conn());
    match action {
        ChainAction::Create { file } => {
            let raw =
                fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
            let def: ChainDefinition = serde_yaml::from_str(&raw)
                .with_context(|| format!("parse YAML {}", file.display()))?;
            let id = store.create(&def).context("INSERT message_chains")?;
            println!("chain created: {} ({})", def.name, id);
            Ok(())
        }
        ChainAction::List => {
            let rows = store.list().context("SELECT message_chains")?;
            if rows.is_empty() {
                println!("(no chains)");
                return Ok(());
            }
            for c in rows {
                println!(
                    "- {name}  id={id}  enabled={enabled}",
                    name = c.name,
                    id = c.id,
                    enabled = c.enabled
                );
            }
            Ok(())
        }
        ChainAction::Show { name } => {
            let (chain, steps) = store
                .get_by_name(&name)
                .with_context(|| format!("chain `{name}` not found"))?;
            println!(
                "chain: {} ({})  enabled={}",
                chain.name, chain.id, chain.enabled
            );
            if let Some(d) = chain.description {
                println!("  description: {d}");
            }
            for s in steps {
                println!(
                    "  [{order}] {kind}:{target}  delay={delay}s  cond={cond:?}",
                    order = s.step_order,
                    kind = s.target_kind.as_str(),
                    target = s.target,
                    delay = s.delay_secs,
                    cond = s.condition_kind.map(|c| c.as_str()),
                );
                println!("       text=`{}`", s.payload);
            }
            Ok(())
        }
        ChainAction::Run { name } => {
            let (chain, steps) = store
                .get_by_name(&name)
                .with_context(|| format!("chain `{name}` not found"))?;
            // channel-mcp-embed 머지 후 RouteEngine 으로 교체.
            let sender = NoopSender;
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = rt.block_on(ChainRunner::run(&steps, &sender, &chain.name));
            println!(
                "chain `{name}` finished. failed={}, steps={}",
                result.failed,
                result.steps.len()
            );
            for s in result.steps {
                let tag = if s.executed {
                    "OK"
                } else if s.skipped_reason.is_some() {
                    "SKIP"
                } else {
                    "FAIL"
                };
                println!(
                    "  [{order}] {tag}  err={err:?} skip={skip:?}",
                    order = s.step_order,
                    err = s.error,
                    skip = s.skipped_reason,
                );
            }
            if result.failed {
                return Err(anyhow!("chain `{name}` failed"));
            }
            Ok(())
        }
        ChainAction::Delete { name } => {
            store
                .delete_by_name(&name)
                .with_context(|| format!("delete chain `{name}`"))?;
            println!("deleted: {name}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        cmd: TestCmd,
    }
    #[derive(clap::Subcommand)]
    enum TestCmd {
        Schedule {
            #[command(subcommand)]
            action: ScheduleAction,
        },
        Chain {
            #[command(subcommand)]
            action: ChainAction,
        },
    }

    #[test]
    fn schedule_once_parses() {
        let cli = TestCli::try_parse_from([
            "x",
            "schedule",
            "once",
            "--at",
            "2099-01-01T09:00:00+09:00",
            "--to-role",
            "res",
            "--text",
            "hi",
        ])
        .unwrap();
        match cli.cmd {
            TestCmd::Schedule {
                action: ScheduleAction::Once { to_role, .. },
            } => assert_eq!(to_role.as_deref(), Some("res")),
            _ => panic!("expected Schedule::Once"),
        }
    }

    #[test]
    fn schedule_cron_parses() {
        let cli = TestCli::try_parse_from([
            "x",
            "schedule",
            "cron",
            "0 9 * * *",
            "--to-platform",
            "discord",
            "--channel-id",
            "12345",
            "--text",
            "standup",
        ])
        .unwrap();
        match cli.cmd {
            TestCmd::Schedule {
                action: ScheduleAction::Cron { cron_expr, .. },
            } => assert_eq!(cron_expr, "0 9 * * *"),
            _ => panic!("expected Schedule::Cron"),
        }
    }

    #[test]
    fn chain_create_parses() {
        let cli =
            TestCli::try_parse_from(["x", "chain", "create", "--file", "/tmp/c.yaml"]).unwrap();
        match cli.cmd {
            TestCmd::Chain {
                action: ChainAction::Create { file },
            } => assert_eq!(file.to_str(), Some("/tmp/c.yaml")),
            _ => panic!(),
        }
    }

    #[test]
    fn chain_run_parses() {
        let cli = TestCli::try_parse_from(["x", "chain", "run", "morning"]).unwrap();
        match cli.cmd {
            TestCmd::Chain {
                action: ChainAction::Run { name },
            } => assert_eq!(name, "morning"),
            _ => panic!(),
        }
    }
}
