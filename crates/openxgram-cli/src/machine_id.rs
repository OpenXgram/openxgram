//! 머신 alias 단일 진리원천(SSOT).
//!
//! 같은 data-dir 를 쓰는 모든 프로세스(daemon · mcp-serve · 세션 등록)는 머신명을
//! **반드시 동일하게** 얻어야 한다. 과거에는 `daemon.rs` / `daemon_gui.rs` 가 각자
//! `XGRAM_MACHINE_ALIAS` env → hostname 으로 폴백하여 derivation 했는데, env 가 없는
//! mcp-serve 프로세스는 hostname(예: `whitegun-win`)으로 갈라져 머신명이 두 개로
//! 분열되는 SSOT 위반이 발생했다.
//!
//! 해결: 머신 alias 를 **data-dir 에 1회 영속된 값**에서 결정한다. 우선순위:
//!   1. 영속된 정본 — install-manifest 의 `machine.alias` (init 이 서명·기록한 정본)
//!   2. (manifest 부재 시) write-once 캐시 파일 `<data_dir>/machine.alias`
//!   3. `XGRAM_MACHINE_ALIAS` env
//!   4. hostname (`detect_machine().alias`)
//!
//! env/hostname 으로 derivation 된 경우, 같은 data-dir 의 후속 프로세스가 동일 값을
//! 얻도록 캐시 파일에 write-once 로 캐시한다(이미 있으면 덮지 않음 — race 안전).

use std::path::Path;

use openxgram_core::paths::manifest_path;

/// data-dir 내부 머신 alias 캐시 파일명. manifest 부재 환경(또는 env/hostname
/// derivation 결과)을 1회 영속하여 같은 data-dir 프로세스 간 일관성 보장.
pub const MACHINE_ALIAS_FILENAME: &str = "machine.alias";

/// `<data_dir>/machine.alias` 경로.
fn machine_alias_cache_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(MACHINE_ALIAS_FILENAME)
}

/// 빈 문자열·공백을 정규화하여 None 으로.
fn non_empty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// 영속된 정본에서 머신 alias 를 읽는다. 우선순위:
///   1. install-manifest 의 `machine.alias`
///   2. write-once 캐시 파일 `<data_dir>/machine.alias`
/// 둘 다 없으면 None.
fn persisted_machine_alias(data_dir: &Path) -> Option<String> {
    // 1) install-manifest — init 이 서명·기록한 정본. 가장 신뢰.
    match openxgram_manifest::InstallManifest::read(manifest_path(data_dir)) {
        Ok(m) => {
            if let Some(alias) = non_empty(m.machine.alias) {
                return Some(alias);
            }
            tracing::debug!("install-manifest 의 machine.alias 가 비어 있음 — 캐시 파일로 폴백");
        }
        Err(e) => {
            // manifest 부재/파손은 정상 폴백 경로(아직 init 안 됨 등) — 조용히 삼키지 않고 debug 로그.
            tracing::debug!(error = %e, "install-manifest 읽기 실패 — machine.alias 캐시 파일로 폴백");
        }
    }

    // 2) write-once 캐시 파일.
    match std::fs::read_to_string(machine_alias_cache_path(data_dir)) {
        Ok(s) => non_empty(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!(error = %e, "machine.alias 캐시 파일 읽기 실패 — env/hostname 폴백");
            None
        }
    }
}

/// env/hostname 으로 derivation 한 머신 alias 를 data-dir 에 write-once 캐시.
/// 이미 파일이 있으면 덮지 않는다(race 안전 — 먼저 쓴 값이 정본).
fn cache_machine_alias_write_once(data_dir: &Path, alias: &str) {
    let path = machine_alias_cache_path(data_dir);
    if path.exists() {
        return;
    }
    // create_new 으로 원자적 write-once — 동시 프로세스가 경합해도 한 번만 쓰여진다.
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(alias.as_bytes()) {
                tracing::warn!(error = %e, path = %path.display(), "machine.alias 캐시 쓰기 실패 (계속)");
            } else {
                tracing::info!(alias = %alias, path = %path.display(), "machine.alias SSOT 캐시 write-once");
            }
        }
        // 다른 프로세스가 먼저 만든 경우 — 정상(write-once 경쟁에서 짐). 무시.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "machine.alias 캐시 파일 생성 실패 (계속)");
        }
    }
}

/// env/hostname 으로 머신 alias derivation. (영속값 부재 시 폴백 경로)
///   1. `XGRAM_MACHINE_ALIAS` env
///   2. hostname (`detect_machine().alias`)
fn derive_from_env_or_hostname() -> String {
    if let Some(alias) = std::env::var("XGRAM_MACHINE_ALIAS")
        .ok()
        .and_then(non_empty)
    {
        return alias;
    }
    crate::daemon_gui_sessions::detect_machine().alias
}

/// 이 머신의 alias — **SSOT**.
///
/// 우선순위: (1) data-dir 영속값(manifest → 캐시 파일) → (2) XGRAM_MACHINE_ALIAS env →
/// (3) hostname. 영속값이 있으면 env/hostname 은 **무시**되어, 같은 data-dir 를 쓰는
/// 모든 프로세스(env 유무·hostname 무관)가 항상 같은 머신명을 얻는다.
///
/// 영속값이 없어 env/hostname 으로 derivation 한 경우, 그 값을 캐시 파일에 write-once
/// 영속하여 후속 프로세스가 동일 값을 얻게 한다.
pub fn machine_alias(data_dir: &Path) -> String {
    if let Some(alias) = persisted_machine_alias(data_dir) {
        return alias;
    }
    let derived = derive_from_env_or_hostname();
    cache_machine_alias_write_once(data_dir, &derived);
    derived
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env 가 process-global 이라 테스트 간 간섭을 막기 위해 직렬화.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_manifest_alias(dir: &Path, alias: &str) {
        // install-manifest 의 machine.alias 만 채운 최소 JSON. read() 는 version 검증만 하므로
        // 정본 SCHEMA_VERSION 을 사용한다.
        let json = serde_json::json!({
            "version": openxgram_manifest::SCHEMA_VERSION,
            "installed_at": "2026-06-20T00:00:00+09:00",
            "machine": {
                "alias": alias,
                "role": "primary",
                "os": "linux",
                "arch": "x86_64",
                "hostname": "irrelevant-host",
                "tailscale_ip": null
            },
            "uninstall_token": ""
        });
        std::fs::write(
            manifest_path(dir),
            serde_json::to_vec_pretty(&json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn persisted_manifest_alias_wins_over_env_and_hostname() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        write_manifest_alias(tmp.path(), "zalman");
        // env 가 다른 값을 가리켜도 영속 manifest 값이 이긴다.
        std::env::set_var("XGRAM_MACHINE_ALIAS", "should-be-ignored");
        assert_eq!(machine_alias(tmp.path()), "zalman");
        std::env::remove_var("XGRAM_MACHINE_ALIAS");
    }

    #[test]
    fn cache_file_used_when_manifest_absent() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // manifest 없음. 캐시 파일만 존재 → 그 값이 env 보다 우선.
        std::fs::write(tmp.path().join(MACHINE_ALIAS_FILENAME), "zalman\n").unwrap();
        std::env::set_var("XGRAM_MACHINE_ALIAS", "should-be-ignored");
        assert_eq!(machine_alias(tmp.path()), "zalman");
        std::env::remove_var("XGRAM_MACHINE_ALIAS");
    }

    #[test]
    fn falls_back_to_env_when_no_persisted_value_and_caches_write_once() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // 영속값 전무 → env 폴백 + write-once 캐시.
        std::env::set_var("XGRAM_MACHINE_ALIAS", "zalman");
        assert_eq!(machine_alias(tmp.path()), "zalman");
        std::env::remove_var("XGRAM_MACHINE_ALIAS");
        // 캐시 파일이 생성되어 후속 호출(env 없어도) 동일 값.
        let cached = std::fs::read_to_string(tmp.path().join(MACHINE_ALIAS_FILENAME)).unwrap();
        assert_eq!(cached.trim(), "zalman");
        assert_eq!(machine_alias(tmp.path()), "zalman");
    }

    #[test]
    fn falls_back_to_hostname_when_no_persisted_value_and_no_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::env::remove_var("XGRAM_MACHINE_ALIAS");
        // env·영속값 전무 → hostname(detect_machine) 폴백. 값 자체는 환경 의존이라
        // 비어있지 않음 + write-once 캐시됨만 검증.
        let got = machine_alias(tmp.path());
        assert!(!got.is_empty());
        let cached = std::fs::read_to_string(tmp.path().join(MACHINE_ALIAS_FILENAME)).unwrap();
        assert_eq!(cached.trim(), got);
    }
}
