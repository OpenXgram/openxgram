use crate::error::DbError;
use crate::migrate::MigrationRunner;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum JournalMode {
    Wal,
    Delete,
    Truncate,
    Persist,
    Memory,
}

impl JournalMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wal => "WAL",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::Persist => "PERSIST",
            Self::Memory => "MEMORY",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DbConfig {
    /// DB 파일 경로 (기본: ~/.openxgram/db.sqlite)
    pub path: PathBuf,
    pub journal_mode: JournalMode,
    /// rusqlite busy_timeout (ms)
    pub busy_timeout_ms: u32,
    pub foreign_keys: bool,
}

impl Default for DbConfig {
    fn default() -> Self {
        let path = openxgram_core::paths::default_data_dir()
            .map(|d| openxgram_core::paths::db_path(&d))
            .unwrap_or_else(|_| PathBuf::from("/tmp/openxgram-db.sqlite"));
        Self {
            path,
            journal_mode: JournalMode::Wal,
            busy_timeout_ms: 5000,
            foreign_keys: true,
        }
    }
}

pub struct Db {
    conn: rusqlite::Connection,
    config: DbConfig,
}

impl Db {
    pub fn open(config: DbConfig) -> Result<Self, DbError> {
        // 1. 부모 디렉토리 생성
        if let Some(parent) = config.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 2. sqlite-vec를 auto_extension으로 등록 (Connection 열기 전에 등록해야 적용됨)
        unsafe {
            type SqliteExtEntryPoint = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut i8,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> i32;
            let ep: SqliteExtEntryPoint =
                std::mem::transmute::<*const (), SqliteExtEntryPoint>(
                    sqlite_vec::sqlite3_vec_init as *const (),
                );
            rusqlite::ffi::sqlite3_auto_extension(Some(ep));
        }

        // 3. Connection 열기 (auto_extension이 자동으로 sqlite-vec 로드)
        let conn = rusqlite::Connection::open(&config.path)?;

        // 4. sqlite-vec 로드 검증
        conn.query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
            .map_err(|e| DbError::VecExtension(format!("vec_version() failed: {e}")))?;

        // 5. PRAGMA 적용
        let mut db = Db { conn, config };
        db.apply_pragmas()?;
        Ok(db)
    }

    fn apply_pragmas(&mut self) -> Result<(), DbError> {
        self.conn
            .pragma_update(None, "journal_mode", self.config.journal_mode.as_str())?;
        self.conn
            .pragma_update(None, "busy_timeout", self.config.busy_timeout_ms as i64)?;
        self.conn
            .pragma_update(None, "foreign_keys", self.config.foreign_keys as i64)?;
        Ok(())
    }

    pub fn migrate(&mut self) -> Result<(), DbError> {
        MigrationRunner::new(&mut self.conn).run_all()
    }

    pub fn conn(&mut self) -> &mut rusqlite::Connection {
        &mut self.conn
    }

    /// `PRAGMA integrity_check` 결과를 반환. 정상 시 `"ok"`.
    pub fn integrity_check(&mut self) -> Result<String, DbError> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok(result)
    }

    /// `schema_migrations` 적용 기록 (version 오름차순).
    pub fn list_applied_migrations(&mut self) -> Result<Vec<crate::MigrationRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT version, name, applied_at FROM schema_migrations ORDER BY version",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(crate::MigrationRecord {
                version: r.get::<_, u32>(0)?,
                name: r.get::<_, String>(1)?,
                applied_at: r.get::<_, String>(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
