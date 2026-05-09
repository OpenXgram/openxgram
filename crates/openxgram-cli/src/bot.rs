//! a) 1머신 N봇 first-class — `xgram bot {add,list,start,stop,link,remove}`.
//!
//! 봇 레지스트리 = `~/.xgram/bots.toml` (TOML, 메모리 룰: 단일 SOT).
//! 봇 한 개 = (name, data_dir, transport_port, gui_port, alias).
//!
//! 라이프사이클은 OS process — `xgram bot start <name>` 가 nohup background spawn,
//! PID 저장. `xgram bot stop <name>` 가 PID kill. systemd 미의존 (그건 운영자용 별도).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const BOTS_TOML: &str = "bots.toml";
const PID_FILE: &str = "bot.pid";

/// `~/.xgram/` 의 root 위치 (XDG_DATA_HOME 우선, fallback HOME).
pub fn xgram_root() -> Result<PathBuf> {
    if let Ok(d) = std::env::var("XGRAM_HOME") {
        return Ok(PathBuf::from(d));
    }
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 없음"))?;
    Ok(PathBuf::from(home).join(".xgram"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BotEntry {
    pub name: String,
    pub data_dir: PathBuf,
    pub transport_port: u16,
    pub gui_port: u16,
    pub alias: String,
    pub created_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BotRegistry {
    #[serde(default)]
    pub bots: Vec<BotEntry>,
}

impl BotRegistry {
    pub fn load(root: &Path) -> Result<Self> {
        let p = root.join(BOTS_TOML);
        if !p.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&p)
            .with_context(|| format!("bots.toml 읽기: {}", p.display()))?;
        let parsed: Self = toml::from_str(&raw).context("bots.toml 파싱")?;
        Ok(parsed)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root)?;
        let p = root.join(BOTS_TOML);
        let body = toml::to_string_pretty(self).context("bots.toml 직렬화")?;
        std::fs::write(&p, body).with_context(|| format!("bots.toml 쓰기: {}", p.display()))?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&BotEntry> {
        self.bots.iter().find(|b| b.name == name)
    }

    pub fn add(&mut self, entry: BotEntry) -> Result<()> {
        if self.get(&entry.name).is_some() {
            bail!("같은 이름 봇 이미 존재: {}", entry.name);
        }
        // 포트 충돌 방지
        for b in &self.bots {
            if b.transport_port == entry.transport_port || b.gui_port == entry.transport_port {
                bail!("transport_port 충돌: {}", entry.transport_port);
            }
            if b.gui_port == entry.gui_port || b.transport_port == entry.gui_port {
                bail!("gui_port 충돌: {}", entry.gui_port);
            }
        }
        self.bots.push(entry);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<BotEntry> {
        let idx = self
            .bots
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| anyhow!("봇 없음: {name}"))?;
        Ok(self.bots.remove(idx))
    }
}

/// 새 봇 등록 — data_dir 생성, init 호출, 포트 자동 할당, 레지스트리 갱신.
/// alias 미지정 시 name 그대로 사용.
pub fn bot_add(name: &str, alias: Option<&str>) -> Result<BotEntry> {
    if name.trim().is_empty() {
        bail!("이름 비어있음");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("이름은 영숫자/-/_ 만 (got: {name})");
    }

    let root = xgram_root()?;
    let mut reg = BotRegistry::load(&root)?;

    let data_dir = root.join("bots").join(name);
    if data_dir.exists() {
        bail!("data_dir 이미 존재: {}", data_dir.display());
    }
    std::fs::create_dir_all(&data_dir).context("data_dir 생성")?;

    let (transport_port, gui_port) = allocate_ports(&reg)?;

    let entry = BotEntry {
        name: name.into(),
        data_dir: data_dir.clone(),
        transport_port,
        gui_port,
        alias: alias.unwrap_or(name).to_string(),
        created_at: openxgram_core::time::kst_now().to_rfc3339(),
    };
    reg.add(entry.clone())?;
    reg.save(&root)?;

    eprintln!("[bot] 등록: {} (alias={}, port={}/{}, dir={})", entry.name, entry.alias, entry.transport_port, entry.gui_port, entry.data_dir.display());
    eprintln!("[bot] 다음: xgram init --data-dir '{}' --alias '{}' (keystore 패스워드 입력)", entry.data_dir.display(), entry.alias);
    eprintln!("[bot] 그 후: xgram bot start {}", entry.name);
    Ok(entry)
}

/// 포트 자동 할당 — 47300 부터 4 단위 (transport+1=gui, transport+2=mcp 영역까지 reserve).
fn allocate_ports(reg: &BotRegistry) -> Result<(u16, u16)> {
    let used: std::collections::HashSet<u16> = reg
        .bots
        .iter()
        .flat_map(|b| {
            // bot start 가 transport+2 를 mcp 로 사용 — 그 영역도 reserve.
            [b.transport_port, b.gui_port, b.transport_port + 2, b.transport_port + 3]
        })
        .collect();
    for base in (47300..47900).step_by(4) {
        let t = base;
        let g = base + 1;
        let mcp = base + 2;
        let spare = base + 3;
        if !used.contains(&t)
            && !used.contains(&g)
            && !used.contains(&mcp)
            && !used.contains(&spare)
        {
            return Ok((t, g));
        }
    }
    bail!("47300-47899 범위 가용 포트 없음")
}

pub fn bot_list() -> Result<()> {
    let root = xgram_root()?;
    let reg = BotRegistry::load(&root)?;
    if reg.bots.is_empty() {
        println!("(등록된 봇 없음)");
        return Ok(());
    }
    println!("{:<20} {:<10} {:<7} {:<7} {:<10} {}", "NAME", "ALIAS", "TPORT", "GPORT", "STATUS", "DATA_DIR");
    for b in &reg.bots {
        let status = if pid_alive(&b.data_dir) { "running" } else { "stopped" };
        println!(
            "{:<20} {:<10} {:<7} {:<7} {:<10} {}",
            b.name,
            b.alias,
            b.transport_port,
            b.gui_port,
            status,
            b.data_dir.display()
        );
    }
    Ok(())
}

pub fn bot_remove(name: &str, force: bool) -> Result<()> {
    let root = xgram_root()?;
    let mut reg = BotRegistry::load(&root)?;
    let entry = reg.remove(name)?;
    if pid_alive(&entry.data_dir) {
        if !force {
            bail!("봇 가동 중. 먼저 `xgram bot stop {name}` 또는 --force");
        }
        let _ = bot_stop(name); // best-effort
    }
    reg.save(&root)?;
    if entry.data_dir.exists() {
        std::fs::remove_dir_all(&entry.data_dir).context("data_dir 삭제")?;
    }
    eprintln!("[bot] 제거: {name}");
    Ok(())
}

/// PID 파일이 있고 그 PID 가 실제로 살아있는지 — 단순 kill -0.
pub fn pid_alive(data_dir: &Path) -> bool {
    let pid_path = data_dir.join(PID_FILE);
    if !pid_path.exists() {
        return false;
    }
    let Ok(s) = std::fs::read_to_string(&pid_path) else { return false };
    let Ok(pid) = s.trim().parse::<i32>() else { return false };
    #[cfg(unix)]
    {
        // signal 0 — alive 체크만, 신호 안 보냄
        unsafe { libc::kill(pid, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// `xgram bot start <name>` — data_dir 의 daemon + MCP HTTP server 를 백그라운드 spawn.
/// MCP 토큰 자동 발급 + .mcp.json 스니펫 출력 (Claude Code/Codex/Cursor 가 attach 가능).
pub fn bot_start(name: &str) -> Result<()> {
    let root = xgram_root()?;
    let reg = BotRegistry::load(&root)?;
    let b = reg.get(name).ok_or_else(|| anyhow!("봇 없음: {name}"))?.clone();
    if pid_alive(&b.data_dir) {
        eprintln!("[bot] {name}: 이미 가동 중 (skip)");
        return Ok(());
    }
    if !b.data_dir.join("install-manifest.json").exists() {
        bail!(
            "봇 미초기화 — 먼저 `XGRAM_KEYSTORE_PASSWORD=... xgram init --alias '{}' --data-dir '{}'` 실행",
            b.alias,
            b.data_dir.display()
        );
    }
    let xgram_bin = std::env::current_exe()
        .or_else(|_| Ok::<_, std::io::Error>(std::path::PathBuf::from("xgram")))
        .unwrap_or_else(|_| std::path::PathBuf::from("xgram"));

    let bind = format!("127.0.0.1:{}", b.transport_port);
    let gui_bind = format!("127.0.0.1:{}", b.gui_port);
    // MCP HTTP 포트 = transport + 2 (gui_port 다음 칸). reservation 은 add 시 가용 영역 확보됨.
    let mcp_port = b.transport_port + 2;
    let mcp_bind = format!("127.0.0.1:{mcp_port}");

    // 1) daemon
    let log_path = b.data_dir.join("bot.log");
    let log_file = std::fs::File::create(&log_path).context("bot.log 생성")?;
    let log_file_clone = log_file.try_clone().context("log fd clone")?;
    let mut daemon_cmd = std::process::Command::new(&xgram_bin);
    daemon_cmd
        .args([
            "daemon",
            "--data-dir",
            &b.data_dir.to_string_lossy(),
            "--bind",
            &bind,
            "--gui-bind",
            &gui_bind,
        ])
        .stdout(log_file)
        .stderr(log_file_clone)
        .stdin(std::process::Stdio::null());
    let daemon_child = daemon_cmd.spawn().context("daemon spawn 실패")?;
    let daemon_pid = daemon_child.id() as i32;
    std::fs::write(b.data_dir.join(PID_FILE), format!("{daemon_pid}\n"))
        .context("PID 파일 쓰기")?;
    std::mem::drop(daemon_child);

    // 2) MCP HTTP server
    let mcp_log_path = b.data_dir.join("mcp.log");
    let mcp_log = std::fs::File::create(&mcp_log_path).context("mcp.log 생성")?;
    let mcp_log_clone = mcp_log.try_clone().context("mcp log fd clone")?;
    let mut mcp_cmd = std::process::Command::new(&xgram_bin);
    mcp_cmd
        .args([
            "mcp-serve",
            "--data-dir",
            &b.data_dir.to_string_lossy(),
            "--bind",
            &mcp_bind,
        ])
        .stdout(mcp_log)
        .stderr(mcp_log_clone)
        .stdin(std::process::Stdio::null());
    let mcp_child = mcp_cmd.spawn().context("mcp-serve spawn 실패")?;
    let mcp_pid = mcp_child.id() as i32;
    std::fs::write(b.data_dir.join("mcp.pid"), format!("{mcp_pid}\n")).ok();
    std::mem::drop(mcp_child);

    // 3) MCP 토큰 자동 발급 (없으면)
    let token = ensure_mcp_token(&b.data_dir, name)?;

    eprintln!("[bot] start: {name}");
    eprintln!("  daemon PID    : {daemon_pid}  (transport http://{bind}, gui http://{gui_bind})");
    eprintln!("  mcp-serve PID : {mcp_pid}     (mcp http://{mcp_bind}/rpc)");
    eprintln!("  data_dir      : {}", b.data_dir.display());
    eprintln!("  log           : {}, {}", log_path.display(), mcp_log_path.display());
    eprintln!();
    eprintln!("Claude Code / Codex / Cursor 의 mcp config 에 추가 (.mcp.json):");
    eprintln!(
        "  {{ \"openxgram-{name}\": {{ \"url\": \"http://{mcp_bind}/rpc\", \"headers\": {{ \"Authorization\": \"Bearer {token}\" }} }} }}"
    );
    Ok(())
}

/// MCP 토큰 발급 (없으면 신규). `~/.xgram/bots/<name>/mcp.token` 0600 으로 보존.
fn ensure_mcp_token(data_dir: &Path, name: &str) -> Result<String> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_core::paths::db_path;

    let token_path = data_dir.join("mcp.token");
    if token_path.exists() {
        let t = std::fs::read_to_string(&token_path)?.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let (_id, token) = crate::mcp_tokens::create_token(&mut db, name, Some("bot start auto"))?;
    std::fs::write(&token_path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&token_path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&token_path, perms)?;
    }
    Ok(token)
}

pub fn bot_stop(name: &str) -> Result<()> {
    let root = xgram_root()?;
    let reg = BotRegistry::load(&root)?;
    let b = reg.get(name).ok_or_else(|| anyhow!("봇 없음: {name}"))?;
    let mut killed_any = false;
    for pid_filename in [PID_FILE, "mcp.pid"] {
        let pid_path = b.data_dir.join(pid_filename);
        if !pid_path.exists() {
            continue;
        }
        let s = std::fs::read_to_string(&pid_path)?;
        if let Ok(pid) = s.trim().parse::<i32>() {
            #[cfg(unix)]
            {
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            killed_any = true;
            eprintln!("[bot] stop: {name}/{pid_filename} (PID {pid} TERM)");
        }
        let _ = std::fs::remove_file(&pid_path);
    }
    if !killed_any {
        eprintln!("[bot] {name}: 가동 중 process 없음 (이미 stopped)");
    }
    Ok(())
}

/// `xgram bot link a b` — 양방향 peer add (a 의 peers 에 b 추가, b 의 peers 에 a 추가).
/// 두 봇 모두 같은 머신에 있어야 함 (다른 머신 봇끼리는 invite QR 사용).
pub fn bot_link(a_name: &str, b_name: &str) -> Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_keystore::{FsKeystore, Keystore};
    use openxgram_peer::{PeerRole, PeerStore};
    use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};

    let root = xgram_root()?;
    let reg = BotRegistry::load(&root)?;
    let a = reg.get(a_name).ok_or_else(|| anyhow!("봇 없음: {a_name}"))?.clone();
    let b = reg.get(b_name).ok_or_else(|| anyhow!("봇 없음: {b_name}"))?.clone();

    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD")
        .context("XGRAM_KEYSTORE_PASSWORD env 필요 — 두 봇이 같은 패스워드 가정")?;

    // a 의 master 정보
    let ks_a = FsKeystore::new(keystore_dir(&a.data_dir));
    let master_a = ks_a.load(MASTER_KEY_NAME, &pw).context("a master 로드")?;
    let pubkey_a = hex::encode(master_a.public_key_bytes());
    let eth_a = master_a.address.to_string();

    // b 의 master 정보
    let ks_b = FsKeystore::new(keystore_dir(&b.data_dir));
    let master_b = ks_b.load(MASTER_KEY_NAME, &pw).context("b master 로드")?;
    let pubkey_b = hex::encode(master_b.public_key_bytes());
    let eth_b = master_b.address.to_string();

    // a 의 DB 에 b peer 추가
    add_peer(&a.data_dir, &b.alias, &pubkey_b, &eth_b, b.transport_port)?;
    // b 의 DB 에 a peer 추가
    add_peer(&b.data_dir, &a.alias, &pubkey_a, &eth_a, a.transport_port)?;

    eprintln!("[bot] link: {a_name} ↔ {b_name} (양방향 peer add)");
    Ok(())
}

fn add_peer(data_dir: &Path, alias: &str, pubkey_hex: &str, eth_addr: &str, port: u16) -> Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_peer::{PeerRole, PeerStore};
    use openxgram_core::paths::db_path;

    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let mut store = PeerStore::new(&mut db);
    let address = format!("http://127.0.0.1:{port}");
    // alias 중복 시 silent skip — 이미 등록된 link 재실행 idempotent.
    if store.get_by_alias(alias)?.is_some() {
        return Ok(());
    }
    let _ = store.add_with_eth(alias, pubkey_hex, &address, Some(eth_addr), PeerRole::Worker, Some("local-bot"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // XGRAM_HOME 은 process-global env — 테스트 병렬 실행 시 충돌. file_serial 로 직렬화.
    fn with_xgram_root<F: FnOnce()>(f: F) {
        let tmp = tempdir().unwrap();
        unsafe { std::env::set_var("XGRAM_HOME", tmp.path()); }
        f();
        unsafe { std::env::remove_var("XGRAM_HOME"); }
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn add_creates_data_dir_and_registry_entry() {
        with_xgram_root(|| {
            let entry = bot_add("alpha", None).unwrap();
            assert_eq!(entry.name, "alpha");
            assert_eq!(entry.alias, "alpha");
            assert!(entry.data_dir.exists());
            assert_eq!(entry.transport_port, 47300);
            assert_eq!(entry.gui_port, 47301);

            // 두 번째 봇 — 4 단위 간격 (mcp 영역까지 reserve)
            let e2 = bot_add("beta", Some("eno")).unwrap();
            assert_eq!(e2.alias, "eno");
            assert_eq!(e2.transport_port, 47304);
            assert_eq!(e2.gui_port, 47305);
        });
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn add_rejects_duplicate_name() {
        with_xgram_root(|| {
            bot_add("dup", None).unwrap();
            let err = bot_add("dup", None).unwrap_err();
            assert!(err.to_string().contains("이미"));
        });
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn add_rejects_invalid_chars() {
        with_xgram_root(|| {
            assert!(bot_add("bad name!", None).is_err());
            assert!(bot_add("", None).is_err());
        });
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn remove_drops_entry_and_data_dir() {
        with_xgram_root(|| {
            let e = bot_add("rem", None).unwrap();
            assert!(e.data_dir.exists());
            bot_remove("rem", false).unwrap();
            assert!(!e.data_dir.exists());
            let reg = BotRegistry::load(&xgram_root().unwrap()).unwrap();
            assert!(reg.get("rem").is_none());
        });
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn list_handles_empty_registry() {
        with_xgram_root(|| {
            bot_list().unwrap();
        });
    }

    #[test]
    #[serial_test::serial(bot_xgram_home)]
    fn registry_round_trip_via_toml() {
        with_xgram_root(|| {
            let root = xgram_root().unwrap();
            let mut reg = BotRegistry::default();
            reg.add(BotEntry {
                name: "rt".into(),
                data_dir: root.join("bots/rt"),
                transport_port: 47300,
                gui_port: 47301,
                alias: "rt".into(),
                created_at: "2026-05-10T00:00:00+09:00".into(),
            }).unwrap();
            reg.save(&root).unwrap();

            let loaded = BotRegistry::load(&root).unwrap();
            assert_eq!(loaded.bots.len(), 1);
            assert_eq!(loaded.bots[0].name, "rt");
        });
    }
}
