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
use chrono::{FixedOffset, Utc};
use openxgram_db::{Db, DbConfig};
use openxgram_manifest::{detect_drift, InstallManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct DoctorOpts {
    pub data_dir: PathBuf,
}

#[derive(Debug)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
}

impl DoctorReport {
    pub fn ok_count(&self) -> usize {
        self.checks.iter().filter(|c| c.verdict == Verdict::Ok).count()
    }
    pub fn warn_count(&self) -> usize {
        self.checks.iter().filter(|c| c.verdict == Verdict::Warn).count()
    }
    pub fn fail_count(&self) -> usize {
        self.checks.iter().filter(|c| c.verdict == Verdict::Fail).count()
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

    pub fn print(&self) {
        let now = Utc::now().with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap());
        println!("xgram doctor 결과 ({})", now.format("%Y-%m-%d %H:%M:%S KST"));
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
    let manifest_path = opts.data_dir.join("install-manifest.json");

    let mut checks = Vec::new();

    let manifest = match InstallManifest::read(&manifest_path) {
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
                detail: format!("{}: {e}", manifest_path.display()),
            });
            None
        }
    };

    checks.push(check_data_dir(&opts.data_dir));
    checks.push(check_sqlite_integrity(&opts.data_dir));
    checks.push(check_keystore(&opts.data_dir));
    checks.push(check_drift(manifest.as_ref()));

    Ok(DoctorReport { checks })
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
    let db_path = data_dir.join("db.sqlite");
    if !db_path.exists() {
        return CheckResult {
            name: "SQLite 무결성",
            verdict: Verdict::Fail,
            detail: format!("DB 파일 미존재: {}", db_path.display()),
        };
    }
    match Db::open(DbConfig {
        path: db_path.clone(),
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
    let path = data_dir.join("keystore").join("master.json");
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
                detail: format!("권한 0o{mode:o} (예상: 600) — `chmod 600 {}`", path.display()),
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
