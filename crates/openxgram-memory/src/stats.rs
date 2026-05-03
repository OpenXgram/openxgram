//! 메모리 레이어 카운트 통계 — status·doctor 에서 공유 사용.

use openxgram_db::Db;

use crate::Result;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub sessions: i64,
    pub messages: i64,
    pub memories: i64,
    pub episodes: i64,
}

pub fn store_stats(db: &mut Db) -> Result<StoreStats> {
    let conn = db.conn();
    let count = |sql: &str| -> rusqlite::Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    Ok(StoreStats {
        sessions: count("SELECT COUNT(*) FROM sessions")?,
        messages: count("SELECT COUNT(*) FROM messages")?,
        memories: count("SELECT COUNT(*) FROM memories")?,
        episodes: count("SELECT COUNT(*) FROM episodes")?,
    })
}
