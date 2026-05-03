//! Per-agent rate limiter (PRD-2.0.4) — 분당 N회.
//!
//! sliding window — 60초 동안 N회 초과 시 reject. agent 미식별 (envelope.from
//! 빈 문자열) 은 글로벌 anonymous 버킷.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

/// 기본 임계값 — `XGRAM_RATE_LIMIT_PER_MIN` env 로 override.
pub const DEFAULT_PER_MIN: u32 = 60;

/// 카운터 윈도우 — 1분 고정.
pub const WINDOW_SECS: u64 = 60;

#[derive(Debug)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
    threshold: u32,
}

impl Default for RateLimiter {
    fn default() -> Self {
        let threshold = std::env::var("XGRAM_RATE_LIMIT_PER_MIN")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_PER_MIN);
        Self {
            buckets: Mutex::new(HashMap::new()),
            threshold,
        }
    }
}

impl RateLimiter {
    pub fn new(threshold: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            threshold,
        }
    }

    /// 호출 1회 등록. threshold 초과 시 Err. 윈도우 밖 entry 는 자동 청소.
    pub fn check_and_record(&self, agent: &str) -> std::result::Result<(), u32> {
        let mut map = self.buckets.lock().expect("poisoned");
        let now = Instant::now();
        let window = std::time::Duration::from_secs(WINDOW_SECS);
        let bucket = map.entry(agent.to_string()).or_default();
        // expired 청소
        while let Some(t) = bucket.front() {
            if now.duration_since(*t) >= window {
                bucket.pop_front();
            } else {
                break;
            }
        }
        if bucket.len() as u32 >= self.threshold {
            return Err(bucket.len() as u32);
        }
        bucket.push_back(now);
        Ok(())
    }

    pub fn threshold(&self) -> u32 {
        self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_threshold_passes() {
        let r = RateLimiter::new(3);
        assert!(r.check_and_record("0xA").is_ok());
        assert!(r.check_and_record("0xA").is_ok());
        assert!(r.check_and_record("0xA").is_ok());
        assert!(r.check_and_record("0xA").is_err());
    }

    #[test]
    fn per_agent_isolation() {
        let r = RateLimiter::new(2);
        assert!(r.check_and_record("0xA").is_ok());
        assert!(r.check_and_record("0xA").is_ok());
        // A 는 차단, B 는 통과
        assert!(r.check_and_record("0xA").is_err());
        assert!(r.check_and_record("0xB").is_ok());
    }
}
