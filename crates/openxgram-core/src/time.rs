//! KST(Asia/Seoul) 시간 헬퍼 — 모든 timestamp 의 단일 source of truth.
//!
//! CLAUDE.md 절대 규칙: 시간대 KST(+09:00). UTC·로컬 timezone 사용 금지.

use chrono::{DateTime, FixedOffset, Utc};

const KST_OFFSET_SECS: i32 = 9 * 3600;

pub fn kst_offset() -> FixedOffset {
    FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST +09:00 offset is always valid")
}

pub fn kst_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&kst_offset())
}
