//! Replay 방지 — (agent, nonce) 단일 사용 + 90초 슬라이딩 윈도우 (PRD-MFA-01).
//!
//! 메모리 캐시 — 데몬 재시작 시 리셋. 90초 보다 오래된 envelope 은 자동 reject
//! (timestamp 검증 별도 함수). 동시 접근은 std Mutex (transport 의 envelope 처리는
//! 1초 1회 batch 라 lock 경합 무시 가능).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// 기본 윈도우 — `XGRAM_REPLAY_WINDOW_SEC` env 로 override.
pub const DEFAULT_WINDOW_SECS: u64 = 90;

#[derive(Debug)]
pub struct ReplayCache {
    seen: Mutex<HashMap<(String, String), Instant>>,
    window: std::time::Duration,
}

impl Default for ReplayCache {
    fn default() -> Self {
        let secs = std::env::var("XGRAM_REPLAY_WINDOW_SEC")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_WINDOW_SECS);
        Self {
            seen: Mutex::new(HashMap::new()),
            window: std::time::Duration::from_secs(secs),
        }
    }
}

impl ReplayCache {
    pub fn new(window: std::time::Duration) -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
            window,
        }
    }

    /// 처음 보는 (agent, nonce) → true (insert 됨). 중복 → false.
    /// 매 호출마다 expired entry 청소.
    pub fn check_and_insert(&self, agent: &str, nonce: &str) -> bool {
        let mut seen = self.seen.lock().expect("poisoned");
        let now = Instant::now();
        // expired entry 제거
        seen.retain(|_, t| now.duration_since(*t) < self.window);
        let key = (agent.to_string(), nonce.to_string());
        if seen.contains_key(&key) {
            return false;
        }
        seen.insert(key, now);
        true
    }

    pub fn len(&self) -> usize {
        self.seen.lock().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_succeeds_then_duplicate_fails() {
        let c = ReplayCache::default();
        assert!(c.check_and_insert("0xA", "n1"));
        assert!(!c.check_and_insert("0xA", "n1"));
        assert!(c.check_and_insert("0xA", "n2"));
        assert!(c.check_and_insert("0xB", "n1"));
    }

    #[test]
    fn expired_entry_evicted() {
        let c = ReplayCache::new(std::time::Duration::from_millis(50));
        assert!(c.check_and_insert("0xA", "n1"));
        std::thread::sleep(std::time::Duration::from_millis(80));
        // expired → 다시 true
        assert!(c.check_and_insert("0xA", "n1"));
    }
}
