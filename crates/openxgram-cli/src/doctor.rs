//! xgram doctor — Phase 1 핵심 점검.
//!
//! SPEC-lifecycle §6 의 첫 PR 범위. 데몬·Embedder·Tailscale·Discord/Telegram·
//! XMTP 항목은 해당 모듈 구현 후 단계적 추가.
//!
//! 점검 항목:
//!   D1. install-manifest read + schema version
//!   D2. 데이터 디렉토리 read/write 가능
//!   D3. SQLite DB 무결성 (PRAGMA integrity_check)
//!   D4. Keystore master.json 존재 + Unix 권한 600
//!   D5. manifest drift 감지 (files/directories/binaries/services/shell)
//!
//! 종료 코드: 0 = 모두 OK, 1 = FAIL 존재, 2 = WARN 만 (FAIL 없음).

use std::path::{Path, PathBuf};

use anyhow::Result;
use openxgram_core::paths::{db_path, manifest_path, master_keyfile};
use openxgram_core::ports::RPC_PORT;
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_manifest::{detect_drift, InstallManifest};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Fail,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Ok => "[OK]  ",
            Self::Warn => "[WARN]",
            Self::Fail => "[FAIL]",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct DoctorOpts {
    pub data_dir: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
}

impl DoctorReport {
    pub fn ok_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::Ok)
            .count()
    }
    pub fn warn_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::Warn)
            .count()
    }
    pub fn fail_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.verdict == Verdict::Fail)
            .count()
    }

    /// 종료 코드: 0 = 모두 OK, 1 = FAIL 존재, 2 = WARN 만.
    pub fn exit_code(&self) -> i32 {
        if self.fail_count() > 0 {
            1
        } else if self.warn_count() > 0 {
            2
        } else {
            0
        }
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "checks": &self.checks,
            "summary": {
                "ok": self.ok_count(),
                "warn": self.warn_count(),
                "fail": self.fail_count(),
            },
        }))?)
    }

    pub fn print(&self) {
        println!(
            "xgram doctor 결과 ({})",
            kst_now().format("%Y-%m-%d %H:%M:%S KST")
        );
        println!();
        for c in &self.checks {
            println!("{} {} — {}", c.verdict, c.name, c.detail);
        }
        println!();
        println!(
            "요약: {} OK, {} WARN, {} FAIL",
            self.ok_count(),
            self.warn_count(),
            self.fail_count()
        );
    }
}

pub fn run_doctor(opts: &DoctorOpts) -> Result<DoctorReport> {
    let mp = manifest_path(&opts.data_dir);

    let mut checks = Vec::new();

    let manifest = match InstallManifest::read(&mp) {
        Ok(m) => {
            checks.push(CheckResult {
                name: "install-manifest",
                verdict: Verdict::Ok,
                detail: format!("schema version {}", m.version),
            });
            Some(m)
        }
        Err(e) => {
            checks.push(CheckResult {
                name: "install-manifest",
                verdict: Verdict::Fail,
                detail: format!("{}: {e}", mp.display()),
            });
            None
        }
    };

    checks.push(check_data_dir(&opts.data_dir));
    checks.push(check_sqlite_integrity(&opts.data_dir));
    checks.push(check_keystore(&opts.data_dir));
    checks.push(check_drift(manifest.as_ref()));
    checks.push(check_transport());
    checks.push(check_memory_layers(&opts.data_dir));
    checks.push(check_vault_layers(&opts.data_dir));
    checks.push(check_embedder_mode());
    checks.push(check_tailscale());

    Ok(DoctorReport { checks })
}

fn check_tailscale() -> CheckResult {
    use openxgram_transport::tailscale;
    if !tailscale::is_running() {
        return CheckResult {
            name: "Tailscale",
            verdict: Verdict::Warn,
            detail: "tailscaled 미실행 또는 미설치 — localhost 전용 (외부 노출 시 `tailscale up` 후 `xgram daemon --tailscale`)".into(),
        };
    }
    let state = tailscale::backend_state().unwrap_or_else(|e| format!("(err: {e})"));
    let ip = tailscale::local_ipv4()
        .map(|a| a.to_string())
        .unwrap_or_else(|e| format!("(err: {e})"));
    if state == "Running" {
        CheckResult {
            name: "Tailscale",
            verdict: Verdict::Ok,
            detail: format!("BackendState=Running, ipv4={ip}"),
        }
    } else {
        CheckResult {
            name: "Tailscale",
            verdict: Verdict::Warn,
            detail: format!("BackendState={state} — `tailscale up` 후 재확인"),
        }
    }
}

fn check_memory_layers(data_dir: &Path) -> CheckResult {
    let path = db_path(data_dir);
    if !path.exists() {
        return CheckResult {
            name: "Memory layers",
            verdict: Verdict::Fail,
            detail: format!("DB 미존재: {}", path.display()),
        };
    }
    let mut db = match Db::open(DbConfig {
        path,
        ..Default::default()
    }) {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Memory layers",
                verdict: Verdict::Fail,
                detail: format!("DB open 실패: {e}"),
            };
        }
    };
    let conn = db.conn();
    let counts = ["messages", "episodes", "memories", "patterns", "traits"]
        .iter()
        .map(|t| {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap_or(-1);
            (t, n)
        })
        .collect::<Vec<_>>();
    let detail = counts
        .iter()
        .map(|(t, n)| format!("{t}={n}"))
        .collect::<Vec<_>>()
        .join(", ");
    CheckResult {
        name: "Memory layers",
        verdict: Verdict::Ok,
        detail,
    }
}

fn check_vault_layers(data_dir: &Path) -> CheckResult {
    let path = db_path(data_dir);
    if !path.exists() {
        return CheckResult {
            name: "Vault layers",
            verdict: Verdict::Fail,
            detail: format!("DB 미존재: {}", path.display()),
        };
    }
    let mut db = match Db::open(DbConfig {
        path,
        ..Default::default()
    }) {
        Ok(d) => d,
        Err(e) => {
            return CheckResult {
                name: "Vault layers",
                verdict: Verdict::Fail,
                detail: format!("DB open 실패: {e}"),
            };
        }
    };
    let conn = db.conn();
    let entries: i64 = conn
        .query_row("SELECT COUNT(*) FROM vault_entries", [], |r| r.get(0))
        .unwrap_or(-1);
    let acl: i64 = conn
        .query_row("SELECT COUNT(*) FROM vault_acl", [], |r| r.get(0))
        .unwrap_or(-1);
    let audit: i64 = conn
        .query_row("SELECT COUNT(*) FROM vault_audit", [], |r| r.get(0))
        .unwrap_or(-1);
    let denied_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM vault_audit
             WHERE allowed = 0 AND timestamp >= date('now', 'localtime')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(-1);

    // 오늘 거부 5건 이상 = WARN — 비정상 호출 패턴 의심
    let verdict = if denied_today >= 5 {
        Verdict::Warn
    } else {
        Verdict::Ok
    };
    CheckResult {
        name: "Vault layers",
        verdict,
        detail: format!("entries={entries}, acl={acl}, audit={audit}, denied_today={denied_today}"),
    }
}

fn check_embedder_mode() -> CheckResult {
    use openxgram_memory::embedder_mode_label;
    match embedder_mode_label() {
        "fastembed" => CheckResult {
            name: "Embedder mode",
            verdict: Verdict::Ok,
            detail: "FastEmbedder (multilingual-e5-small) 활성".to_string(),
        },
        "fastembed-overridden-dummy" => CheckResult {
            name: "Embedder mode",
            verdict: Verdict::Warn,
            detail: "fastembed 빌드되어 있으나 XGRAM_EMBEDDER=dummy 로 비활성".to_string(),
        },
        _ => CheckResult {
            name: "Embedder mode",
            verdict: Verdict::Warn,
            detail:
                "DummyEmbedder (CI/test 결정성) — `--features fastembed` 빌드 시 의미 임베딩 활성"
                    .to_string(),
        },
    }
}

fn check_transport() -> CheckResult {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = ([127, 0, 0, 1], RPC_PORT).into();
    match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        Ok(_) => CheckResult {
            name: "Transport server",
            verdict: Verdict::Ok,
            detail: format!("127.0.0.1:{RPC_PORT} 연결 성공 (daemon up)"),
        },
        Err(_) => CheckResult {
            name: "Transport server",
            verdict: Verdict::Warn,
            detail: format!(
                "127.0.0.1:{RPC_PORT} 연결 실패 (daemon 미실행 — `xgram daemon` 으로 시작)"
            ),
        },
    }
}

fn check_data_dir(data_dir: &Path) -> CheckResult {
    if !data_dir.exists() {
        return CheckResult {
            name: "데이터 디렉토리",
            verdict: Verdict::Fail,
            detail: format!("미존재: {}", data_dir.display()),
        };
    }
    let probe = data_dir.join(".xgram-doctor-write-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            CheckResult {
                name: "데이터 디렉토리",
                verdict: Verdict::Ok,
                detail: format!("read/write 가능: {}", data_dir.display()),
            }
        }
        Err(e) => CheckResult {
            name: "데이터 디렉토리",
            verdict: Verdict::Fail,
            detail: format!("쓰기 실패 {}: {e}", data_dir.display()),
        },
    }
}

fn check_sqlite_integrity(data_dir: &Path) -> CheckResult {
    let path = db_path(data_dir);
    if !path.exists() {
        return CheckResult {
            name: "SQLite 무결성",
            verdict: Verdict::Fail,
            detail: format!("DB 파일 미존재: {}", path.display()),
        };
    }
    match Db::open(DbConfig {
        path: path.clone(),
        ..Default::default()
    }) {
        Ok(mut db) => match db.integrity_check() {
            Ok(s) if s == "ok" => CheckResult {
                name: "SQLite 무결성",
                verdict: Verdict::Ok,
                detail: "PRAGMA integrity_check = ok".into(),
            },
            Ok(other) => CheckResult {
                name: "SQLite 무결성",
                verdict: Verdict::Fail,
                detail: format!("PRAGMA integrity_check = {other}"),
            },
            Err(e) => CheckResult {
                name: "SQLite 무결성",
                verdict: Verdict::Fail,
                detail: format!("PRAGMA 실행 실패: {e}"),
            },
        },
        Err(e) => CheckResult {
            name: "SQLite 무결성",
            verdict: Verdict::Fail,
            detail: format!("DB open 실패: {e}"),
        },
    }
}

fn check_keystore(data_dir: &Path) -> CheckResult {
    let path = master_keyfile(data_dir);
    if !path.exists() {
        return CheckResult {
            name: "Keystore master",
            verdict: Verdict::Fail,
            detail: format!("미존재: {}", path.display()),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                return CheckResult {
                    name: "Keystore master",
                    verdict: Verdict::Fail,
                    detail: format!("metadata 실패: {e}"),
                };
            }
        };
        let mode = meta.permissions().mode() & 0o777;
        if mode == 0o600 {
            CheckResult {
                name: "Keystore master",
                verdict: Verdict::Ok,
                detail: format!("권한 600 ({})", path.display()),
            }
        } else {
            CheckResult {
                name: "Keystore master",
                verdict: Verdict::Warn,
                detail: format!(
                    "권한 0o{mode:o} (예상: 600) — `chmod 600 {}`",
                    path.display()
                ),
            }
        }
    }

    #[cfg(not(unix))]
    {
        CheckResult {
            name: "Keystore master",
            verdict: Verdict::Ok,
            detail: format!("존재 ({}) — Unix 권한 검사 건너뜀", path.display()),
        }
    }
}

fn check_drift(manifest: Option<&InstallManifest>) -> CheckResult {
    let Some(m) = manifest else {
        return CheckResult {
            name: "manifest drift",
            verdict: Verdict::Fail,
            detail: "manifest 로드 실패로 검사 불가".into(),
        };
    };
    let drift = detect_drift(m);
    if drift.is_empty() {
        CheckResult {
            name: "manifest drift",
            verdict: Verdict::Ok,
            detail: "변조·누락 없음".into(),
        }
    } else {
        CheckResult {
            name: "manifest drift",
            verdict: Verdict::Fail,
            detail: format!("{}건 발견 (자세한 항목은 후속 보고)", drift.len()),
        }
    }
}
