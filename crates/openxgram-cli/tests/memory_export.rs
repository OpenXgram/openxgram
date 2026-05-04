//! xgram memory export/import — Claude 호환 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::memory::{run_export, run_import, run_memory, MemoryAction, MemoryExportFmt};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_manifest::MachineRole;
use openxgram_memory::{export_claude, MemoryKind, TraitSource, TraitStore};
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-export";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "memory-export-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    // SAFETY: 통합 테스트 격리 — file_serial 로 직렬화.
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
}

fn open_db(data_dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

#[test]
#[serial_test::file_serial]
fn export_then_import_roundtrip() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();

    // L2 memory 1개 (Rule → Instructions 카테고리에 매핑)
    run_memory(
        &data_dir,
        MemoryAction::Add {
            kind: MemoryKind::Rule,
            content: "표(table) 사용 금지 — 마크다운 목록으로 정리".into(),
            session_id: None,
        },
    )
    .unwrap();

    // L2 memory 1개 (Reference → Preferences)
    run_memory(
        &data_dir,
        MemoryAction::Add {
            kind: MemoryKind::Reference,
            content: "한국어로 출력".into(),
            session_id: None,
        },
    )
    .unwrap();

    // L4 trait 1개 직접 insert
    {
        let mut db = open_db(&data_dir);
        let mut store = TraitStore::new(&mut db);
        store
            .insert_or_update(
                "tone:concise",
                "응답은 짧고 정확하게",
                TraitSource::Manual,
                &[],
            )
            .unwrap();
    }

    // export → markdown 검증
    let out_path = tmp.path().join("export.md");
    run_export(&data_dir, Some(&out_path), MemoryExportFmt::Claude).unwrap();
    let md = std::fs::read_to_string(&out_path).unwrap();
    assert!(md.starts_with("```\n"), "code block 헤더 누락: {md}");
    assert!(md.trim_end().ends_with("```"), "code block 종결 누락");
    assert!(
        md.contains("## Instructions"),
        "Instructions 카테고리 헤더 누락: {md}"
    );
    assert!(
        md.contains("표(table) 사용 금지"),
        "memory entry 본문 누락: {md}"
    );

    // 별도 신규 데이터 디렉토리에 init 후 import → 카운트 검증
    let tmp2 = tempdir().unwrap();
    let data_dir2 = tmp2.path().join("openxgram");
    let mut opts2 = init_opts(data_dir2.clone());
    opts2.alias = "memory-export-test-2".into();
    run_init(&opts2).unwrap();

    run_import(&data_dir2, &out_path, MemoryExportFmt::Claude).unwrap();

    // import 결과 — memories/traits 가 1개 이상 들어와야 한다.
    let mut db2 = open_db(&data_dir2);
    let exp2 = export_claude(&mut db2).unwrap();
    let total_entries: usize = exp2.buckets.values().map(|v| v.len()).sum();
    assert!(
        total_entries >= 2,
        "import 후 entry 수 부족 (기대 ≥ 2, 실제 {total_entries})"
    );
}
