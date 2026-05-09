//! systemd user unit 생성기 — xgram daemon 을 백그라운드로 띄우기 위한
//! `~/.config/systemd/user/openxgram-sidecar.service` 작성.
//!
//! Phase 1: install / uninstall 만. ExecStart 의 binary 경로는 인자로 받음
//! (기본 `which xgram` 결과). 환경변수(XGRAM_KEYSTORE_PASSWORD)는 사용자가
//! 별도 systemd-creds 또는 EnvironmentFile 로 주입.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const UNIT_FILENAME: &str = "openxgram-sidecar.service";
const AGENT_UNIT_FILENAME: &str = "openxgram-agent.service";
const BACKUP_SERVICE_FILENAME: &str = "openxgram-backup.service";
const BACKUP_TIMER_FILENAME: &str = "openxgram-backup.timer";

/// EnvironmentFile 권장 위치 — systemd-creds 패턴이 어려우면 이 파일에서 읽도록.
/// 0600 퍼미션으로 작성 권장 (XGRAM_KEYSTORE_PASSWORD 등 비밀 포함).
pub const DEFAULT_ENV_FILE: &str = "openxgram.env";

/// 기본 OnCalendar — 매주 일요일 03:00 KST. systemd 가 로컬 timezone 기준 처리.
pub const DEFAULT_BACKUP_ON_CALENDAR: &str = "Sun 03:00:00";

#[derive(Debug, Clone)]
pub struct UnitOpts {
    /// xgram binary 절대 경로
    pub binary: PathBuf,
    /// daemon 데이터 디렉토리 (--data-dir 인자)
    pub data_dir: PathBuf,
    /// transport bind 주소
    pub bind: String,
}

pub fn default_user_unit_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 누락"))?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(UNIT_FILENAME))
}

pub fn render_unit(opts: &UnitOpts) -> String {
    format!(
        "# OpenXgram systemd user unit\n\
[Unit]\n\
Description=OpenXgram sidecar daemon\n\
After=network.target\n\
\n\
[Service]\n\
Type=simple\n\
ExecStart={binary} daemon --data-dir {data_dir} --bind {bind}\n\
Restart=on-failure\n\
RestartSec=5\n\
\n\
[Install]\n\
WantedBy=default.target\n",
        binary = opts.binary.display(),
        data_dir = opts.data_dir.display(),
        bind = opts.bind,
    )
}

pub fn install_user_unit(target: &Path, opts: &UnitOpts) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    if target.exists() {
        bail!(
            "unit 파일 이미 존재: {} — 먼저 uninstall 실행하거나 다른 경로 지정",
            target.display()
        );
    }
    std::fs::write(target, render_unit(opts))
        .with_context(|| format!("unit 파일 저장 실패: {}", target.display()))?;
    Ok(())
}

pub fn uninstall_user_unit(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    std::fs::remove_file(target)
        .with_context(|| format!("unit 파일 제거 실패: {}", target.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BackupUnitOpts {
    /// xgram binary 절대 경로
    pub binary: PathBuf,
    /// 데이터 디렉토리
    pub data_dir: PathBuf,
    /// cold backup 출력 디렉토리 (timestamped 파일 생성됨)
    pub backup_dir: PathBuf,
    /// systemd OnCalendar 표현식 (기본 DEFAULT_BACKUP_ON_CALENDAR)
    pub on_calendar: String,
}

pub fn default_backup_service_path() -> Result<PathBuf> {
    Ok(default_user_unit_path()?
        .parent()
        .ok_or_else(|| anyhow!("user unit 부모 경로 누락"))?
        .join(BACKUP_SERVICE_FILENAME))
}

pub fn default_backup_timer_path() -> Result<PathBuf> {
    Ok(default_user_unit_path()?
        .parent()
        .ok_or_else(|| anyhow!("user unit 부모 경로 누락"))?
        .join(BACKUP_TIMER_FILENAME))
}

pub fn render_backup_service(opts: &BackupUnitOpts) -> String {
    format!(
        "# OpenXgram cold backup oneshot — invoked by openxgram-backup.timer\n\
[Unit]\n\
Description=OpenXgram cold backup\n\
\n\
[Service]\n\
Type=oneshot\n\
ExecStart={binary} backup --data-dir {data_dir} --to {backup_dir}\n",
        binary = opts.binary.display(),
        data_dir = opts.data_dir.display(),
        backup_dir = opts.backup_dir.display(),
    )
}

pub fn render_backup_timer(opts: &BackupUnitOpts) -> String {
    format!(
        "# OpenXgram cold backup timer (KST 기준 OnCalendar — systemd 가 로컬 tz 사용)\n\
[Unit]\n\
Description=OpenXgram cold backup timer\n\
\n\
[Timer]\n\
OnCalendar={on_calendar}\n\
Persistent=true\n\
Unit=openxgram-backup.service\n\
\n\
[Install]\n\
WantedBy=timers.target\n",
        on_calendar = opts.on_calendar,
    )
}

/// service + timer 두 파일을 동시 작성. 둘 중 하나라도 이미 있으면 raise.
pub fn install_backup_units(
    service_path: &Path,
    timer_path: &Path,
    opts: &BackupUnitOpts,
) -> Result<()> {
    if let Some(parent) = service_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    for p in [service_path, timer_path] {
        if p.exists() {
            bail!(
                "unit 파일 이미 존재: {} — 먼저 backup-uninstall 실행",
                p.display()
            );
        }
    }
    std::fs::write(service_path, render_backup_service(opts))
        .with_context(|| format!("service 저장 실패: {}", service_path.display()))?;
    std::fs::write(timer_path, render_backup_timer(opts))
        .with_context(|| format!("timer 저장 실패: {}", timer_path.display()))?;
    Ok(())
}

pub fn uninstall_backup_units(service_path: &Path, timer_path: &Path) -> Result<()> {
    for p in [service_path, timer_path] {
        if p.exists() {
            std::fs::remove_file(p)
                .with_context(|| format!("unit 파일 제거 실패: {}", p.display()))?;
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 6.1.1 — agent unit (xgram agent 백그라운드 가동) + 6.1.1.2 — systemd-creds 설계
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentUnitOpts {
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    /// EnvironmentFile 경로 (XGRAM_KEYSTORE_PASSWORD / DISCORD/TELEGRAM/ANTHROPIC 토큰 들어있는 파일).
    /// None 이면 EnvironmentFile 행 생략 — 사용자가 외부에서 주입.
    pub environment_file: Option<PathBuf>,
}

pub fn default_agent_unit_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 누락"))?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(AGENT_UNIT_FILENAME))
}

pub fn render_agent_unit(opts: &AgentUnitOpts) -> String {
    let env_line = match &opts.environment_file {
        Some(p) => format!("EnvironmentFile=-{}\n", p.display()),
        None => String::new(),
    };
    format!(
        "# OpenXgram agent runtime — inbox 폴링 + LLM 응답 + 채널 forward\n\
[Unit]\n\
Description=OpenXgram agent runtime\n\
After=network.target openxgram-sidecar.service\n\
Wants=openxgram-sidecar.service\n\
\n\
[Service]\n\
Type=simple\n\
{env_line}\
ExecStart={binary} agent --data-dir {data_dir}\n\
Restart=on-failure\n\
RestartSec=5\n\
\n\
[Install]\n\
WantedBy=default.target\n",
        binary = opts.binary.display(),
        data_dir = opts.data_dir.display(),
    )
}

pub fn install_agent_unit(target: &Path, opts: &AgentUnitOpts) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    if target.exists() {
        bail!(
            "agent unit 파일 이미 존재: {} — 먼저 uninstall 실행",
            target.display()
        );
    }
    std::fs::write(target, render_agent_unit(opts))
        .with_context(|| format!("agent unit 저장 실패: {}", target.display()))?;
    Ok(())
}

pub fn uninstall_agent_unit(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    std::fs::remove_file(target)
        .with_context(|| format!("agent unit 제거 실패: {}", target.display()))?;
    Ok(())
}

/// 6.1.1.2 — systemd-creds 또는 EnvironmentFile 작성 (chmod 0600).
/// `entries` 의 각 (key, value) 가 `KEY=VALUE` 형식 한 줄.
/// 동일 파일 존재 시 덮어쓰기 — 시크릿 회전 가능.
pub fn write_environment_file<I, K, V>(path: &Path, entries: I) -> Result<()>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("env 부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    let mut buf = String::from("# OpenXgram systemd EnvironmentFile (write 0600)\n");
    for (k, v) in entries {
        let k = k.as_ref();
        let v = v.as_ref();
        if k.is_empty() || k.contains('=') || k.contains('\n') {
            bail!("invalid env key: {k:?}");
        }
        if v.contains('\n') {
            bail!("env value 한 줄 강제: {k}");
        }
        buf.push_str(&format!("{k}={v}\n"));
    }
    std::fs::write(path, &buf)
        .with_context(|| format!("env 파일 저장 실패: {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn render_agent_unit_includes_env_file_when_provided() {
        let opts = AgentUnitOpts {
            binary: PathBuf::from("/usr/local/bin/xgram"),
            data_dir: PathBuf::from("/var/lib/openxgram"),
            environment_file: Some(PathBuf::from("/etc/openxgram/openxgram.env")),
        };
        let rendered = render_agent_unit(&opts);
        assert!(rendered.contains("EnvironmentFile=-/etc/openxgram/openxgram.env"));
        assert!(rendered.contains("ExecStart=/usr/local/bin/xgram agent --data-dir /var/lib/openxgram"));
        assert!(rendered.contains("After=network.target openxgram-sidecar.service"));
    }

    #[test]
    fn render_agent_unit_omits_env_file_when_none() {
        let opts = AgentUnitOpts {
            binary: PathBuf::from("/usr/local/bin/xgram"),
            data_dir: PathBuf::from("/var/lib/openxgram"),
            environment_file: None,
        };
        let rendered = render_agent_unit(&opts);
        assert!(!rendered.contains("EnvironmentFile="));
    }

    #[test]
    fn install_then_uninstall_agent_unit_round_trip() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("openxgram-agent.service");
        install_agent_unit(
            &target,
            &AgentUnitOpts {
                binary: PathBuf::from("/usr/local/bin/xgram"),
                data_dir: tmp.path().to_path_buf(),
                environment_file: None,
            },
        )
        .unwrap();
        assert!(target.exists());
        // 두 번 install 은 raise
        assert!(install_agent_unit(
            &target,
            &AgentUnitOpts {
                binary: PathBuf::from("/x"),
                data_dir: tmp.path().to_path_buf(),
                environment_file: None,
            }
        )
        .is_err());
        uninstall_agent_unit(&target).unwrap();
        assert!(!target.exists());
        // idempotent uninstall
        uninstall_agent_unit(&target).unwrap();
    }

    #[test]
    fn env_file_written_with_0600_perms_on_unix() {
        let tmp = tempdir().unwrap();
        let env = tmp.path().join("openxgram.env");
        write_environment_file(
            &env,
            [
                ("XGRAM_KEYSTORE_PASSWORD", "secret"),
                ("XGRAM_DISCORD_BOT_TOKEN", "tok"),
            ],
        )
        .unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert!(body.contains("XGRAM_KEYSTORE_PASSWORD=secret"));
        assert!(body.contains("XGRAM_DISCORD_BOT_TOKEN=tok"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&env).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "EnvironmentFile 은 0600");
        }
    }

    #[test]
    fn env_file_rejects_invalid_keys() {
        let tmp = tempdir().unwrap();
        let env = tmp.path().join("bad.env");
        let bad = vec![("BAD=KEY", "v")];
        let res = write_environment_file(&env, bad);
        assert!(res.is_err());
    }

    #[test]
    fn agent_unit_path_default_under_user_systemd() {
        unsafe { std::env::set_var("HOME", "/home/test"); }
        let p = default_agent_unit_path().unwrap();
        assert!(p.ends_with(".config/systemd/user/openxgram-agent.service"));
    }
}
