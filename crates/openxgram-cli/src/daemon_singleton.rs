//! 데이터 디렉토리당 데몬 1개 — 코어 데몬 싱글톤 가드.
//!
//! 같은 `data_dir` 에 두 번째 `xgram daemon` 이 떠서 포트 충돌·이중 발화 같은
//! 운영 사고가 나는 것을 원천 차단한다. bot.rs 의 `bot.pid` 컨벤션을 코어
//! 데몬용으로 이식하되, race 를 막기 위해 advisory flock 을 추가로 건다.
//!
//! 메커니즘 (data_dir 단위, 다른 봇 데몬에는 절대 영향 없음):
//!   1) `<data_dir>/daemon.lock` 을 열고 `flock(LOCK_EX|LOCK_NB)` 비차단 획득.
//!      - 이미 다른 살아있는 데몬이 잡고 있으면 즉시 실패 → race-free.
//!      - flock 은 fd close(=프로세스 종료) 시 커널이 자동 해제 → stale lock 없음.
//!   2) lock 획득 후 `<data_dir>/daemon.pid` 에서 기존 pid liveness 확인.
//!      - flock 미지원 환경(이론상)·옛 데몬 잔재 대비 2차 방어.
//!   3) 자기 pid 를 `daemon.pid` 에 기록.
//!
//! production 규칙: fallback 금지 — 모든 오류는 명시 에러로 반환/로깅한다.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const PID_FILE: &str = "daemon.pid";
const LOCK_FILE: &str = "daemon.lock";

/// 살아있는 데몬이 점유 중인 data_dir 에서 두 번째 기동을 시도했을 때의 에러.
/// `run_daemon` 이 이 에러로 비정상(non-zero) 종료한다.
/// (openxgram-cli 는 thiserror 직접 의존이 없어 Display/Error 를 수동 구현.)
#[derive(Debug)]
pub struct AlreadyRunning {
    pub pid: i32,
    pub dir: String,
}

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[daemon] 이미 실행 중 (pid {}, data-dir {}) — 중복 기동을 중단합니다",
            self.pid, self.dir
        )
    }
}

impl std::error::Error for AlreadyRunning {}

/// 싱글톤 락 가드. 살아있는 동안(=fd 가 열려있는 동안) flock 을 보유한다.
/// `run_daemon` 이 함수 스코프 전체에서 이 가드를 들고 있어야 락이 유지된다.
/// Drop 시 best-effort 로 pidfile 을 제거하고, fd close 로 flock 이 해제된다.
#[derive(Debug)]
pub struct DaemonLock {
    pid_path: PathBuf,
    // flock 보유용 fd. Drop 까지 살아있어야 한다(필드로 보관 — close 금지).
    #[cfg(unix)]
    _lock_file: std::fs::File,
}

impl DaemonLock {
    /// data_dir 기준 싱글톤 락을 획득한다. 포트 바인딩 *전에* 호출할 것.
    ///
    /// - 빈/정상 data_dir: 락 획득 + 자기 pid 기록 후 가드 반환.
    /// - 살아있는 데몬 점유 중: `AlreadyRunning` 에러 반환(기존 데몬은 건드리지 않음).
    /// - stale pidfile(죽은 pid): 정상으로 보고 reclaim(덮어쓰기) 후 진행.
    pub fn acquire(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("data_dir 생성 실패: {}", data_dir.display()))?;
        let pid_path = data_dir.join(PID_FILE);
        let lock_path = data_dir.join(LOCK_FILE);

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&lock_path)
                .with_context(|| format!("daemon.lock 열기 실패: {}", lock_path.display()))?;

            // 비차단 배타 락 — 이미 다른 데몬이 잡고 있으면 즉시 EWOULDBLOCK.
            let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                // EWOULDBLOCK/EAGAIN = 다른 살아있는 데몬이 점유 중 → AlreadyRunning.
                if matches!(err.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK) {
                    let pid = read_pid(&pid_path).unwrap_or(-1);
                    return Err(anyhow::Error::new(AlreadyRunning {
                        pid,
                        dir: data_dir.display().to_string(),
                    }));
                }
                // 그 외 flock 오류는 조용히 삼키지 않고 명시 에러로 전파.
                return Err(err)
                    .with_context(|| format!("daemon.lock flock 실패: {}", lock_path.display()));
            }

            // flock 획득 성공. 2차 방어 — stale 이 아닌 살아있는 pid 가 있으면 중복으로 간주.
            // (정상적으로는 flock 이 먼저 걸러내지만, flock 미지원 FS 대비 명시 확인.)
            if let Some(pid) = read_pid(&pid_path) {
                if pid_alive(pid) && pid != std::process::id() as i32 {
                    return Err(anyhow::Error::new(AlreadyRunning {
                        pid,
                        dir: data_dir.display().to_string(),
                    }));
                }
                // 죽은 pid → stale. reclaim 진행(아래 write 로 덮어씀).
            }

            write_pid(&pid_path)?;
            tracing::info!(
                pid = std::process::id(),
                data_dir = %data_dir.display(),
                "daemon 싱글톤 락 획득"
            );
            Ok(Self {
                pid_path,
                _lock_file: lock_file,
            })
        }

        #[cfg(not(unix))]
        {
            let _ = lock_path;
            // 비-unix: flock 없이 pidfile liveness 만으로 가드(2차 방어 동일 로직).
            if let Some(pid) = read_pid(&pid_path) {
                if pid_alive(pid) && pid != std::process::id() as i32 {
                    return Err(anyhow::Error::new(AlreadyRunning {
                        pid,
                        dir: data_dir.display().to_string(),
                    }));
                }
            }
            write_pid(&pid_path)?;
            Ok(Self { pid_path })
        }
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        // best-effort pidfile 제거 — 없어도 stale 처리로 안전하니 실패해도 무방.
        // flock 은 fd(_lock_file) close 로 커널이 자동 해제한다.
        if let Err(e) = std::fs::remove_file(&self.pid_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, path = %self.pid_path.display(), "daemon.pid 제거 실패(무시)");
            }
        }
    }
}

/// pidfile 에서 pid 파싱. 파일 없음/형식 불량은 None(=점유자 없음으로 취급).
fn read_pid(pid_path: &Path) -> Option<i32> {
    let s = std::fs::read_to_string(pid_path).ok()?;
    s.trim().parse::<i32>().ok()
}

/// 자기 pid 를 pidfile 에 기록.
fn write_pid(pid_path: &Path) -> Result<()> {
    std::fs::write(pid_path, format!("{}\n", std::process::id()))
        .with_context(|| format!("daemon.pid 쓰기 실패: {}", pid_path.display()))
}

/// pid liveness — bot.rs::pid_alive 와 동일 방식(kill -0). 신호는 안 보낸다.
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // signal 0 — 존재/권한 확인만, 실제 신호 미전송.
        unsafe { libc::kill(pid, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_on_empty_dir_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock = match DaemonLock::acquire(tmp.path()) {
            Ok(l) => l,
            Err(e) => panic!("빈 data_dir 락 획득 실패: {e:#}"),
        };
        // pidfile 에 자기 pid 가 기록되어야 한다.
        let pid = read_pid(&tmp.path().join(PID_FILE)).expect("pidfile");
        assert_eq!(pid, std::process::id() as i32);
        drop(lock);
        // Drop 후 pidfile 제거.
        assert!(read_pid(&tmp.path().join(PID_FILE)).is_none());
    }

    #[test]
    fn second_acquire_while_held_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _first = match DaemonLock::acquire(tmp.path()) {
            Ok(l) => l,
            Err(e) => panic!("첫 락 실패: {e:#}"),
        };
        // 같은 data_dir 두 번째 획득 — flock 점유 중이라 실패해야 한다.
        let err = DaemonLock::acquire(tmp.path()).expect_err("두 번째 락은 실패해야 함");
        let already = err
            .downcast_ref::<AlreadyRunning>()
            .expect("AlreadyRunning 타입");
        assert!(already.pid >= -1);
    }

    #[test]
    fn stale_pidfile_is_reclaimed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // 절대 살아있지 않은 pid(2^31-1 부근, 시스템에 존재 불가)로 stale pidfile 작성.
        // flock 파일은 없으므로 flock 은 즉시 획득되고, pidfile 은 죽은 pid → reclaim.
        let dead_pid: i32 = i32::MAX - 1;
        assert!(
            !pid_alive(dead_pid),
            "테스트 전제: dead_pid 가 죽어있어야 함"
        );
        std::fs::write(tmp.path().join(PID_FILE), format!("{dead_pid}\n")).expect("stale pid 작성");

        let lock = match DaemonLock::acquire(tmp.path()) {
            Ok(l) => l,
            Err(e) => panic!("stale pidfile → reclaim 실패: {e:#}"),
        };
        let pid = read_pid(&tmp.path().join(PID_FILE)).expect("pidfile");
        assert_eq!(pid, std::process::id() as i32, "자기 pid 로 덮어써야 함");
        drop(lock);
    }
}
