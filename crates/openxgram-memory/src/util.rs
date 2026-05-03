//! 모듈 내부 공통 헬퍼 — bytes 변환, timestamp 파싱.

use chrono::{DateTime, FixedOffset};

use crate::MemoryError;

pub(crate) fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

pub(crate) fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>, MemoryError> {
    DateTime::parse_from_rfc3339(s).map_err(|e| MemoryError::InvalidTimestamp(e.to_string()))
}
