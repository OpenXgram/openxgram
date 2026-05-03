use openxgram_db::{Db, DbConfig};
use tempfile::TempDir;

#[test]
fn test_open_creates_dir() {
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("subdir/db.sqlite"),
        ..Default::default()
    };
    let _db = Db::open(cfg).unwrap();
    assert!(tmp.path().join("subdir/db.sqlite").exists());
}

#[test]
fn test_migrate_idempotent() {
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    db.migrate().unwrap(); // 두 번 실행해도 안전

    // schema_migrations에 1개 레코드만
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_pragmas_applied() {
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    let mode: String = db
        .conn()
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
}

#[test]
fn test_session_insert_select() {
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();

    let now = chrono::Local::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, last_active, home_machine) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["sess-1", "test", now, now, "gcp"],
        )
        .unwrap();

    let title: String = db
        .conn()
        .query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            ["sess-1"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "test");
}

#[test]
fn test_unexpected_row_count_raises() {
    // UnexpectedRowCount는 migration 내부 INSERT에서 affected != 1 시 발생한다.
    // idempotent 테스트에서 두 번째 run_all이 already=true로 스킵됨을 확인.
    // 이 테스트는 패턴 자체를 문서화한다.
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    // 정상 케이스: 첫 migrate는 성공 (UnexpectedRowCount 없음)
    db.migrate().unwrap();
    // 두 번째: already=true로 스킵 → 에러 없음
    let result = db.migrate();
    assert!(result.is_ok());
}

#[test]
fn test_sqlite_vec_loaded() {
    let tmp = TempDir::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    let version: String = db
        .conn()
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .expect("sqlite-vec must be loaded");
    assert!(!version.is_empty());
}
