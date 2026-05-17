# openxgram-wiki

L2 위키 페이지 (Karpathy 패턴) — 디스크 markdown + DB 인덱스 동기화.

> 정본: [`docs/PRD-OpenXgram.md` §4.1](../../../docs/PRD-OpenXgram.md)

## 핵심 원칙

- **디스크가 정본**. 사용자가 직접 열람·수정 가능한 markdown 파일.
- **DB는 인덱스**. 검색·KNN·관련 페이지 조회용. content_hash로 동기화.
- **충돌 = 에러**. silent fallback 금지. content_hash 불일치는 `WikiError::ContentHashMismatch`로 노출.

## 디렉토리

```
{XGRAM_DATA_DIR}/wiki/
├── entity/        ← 사람·프로젝트·도구
├── concept/       ← 방법론·패턴
├── comparison/    ← A vs B
└── other/         ← 그 외
```

## MCP 도구 5개 (PRD §4.1)

- `read_wiki_page(topic)` — 페이지 본문 + frontmatter
- `write_wiki_page(topic, content, type?)` — 생성/업데이트 (낙관 잠금)
- `link_concepts(from, to, reason?)` — 크로스링크
- `search_wiki(query, k?=5)` — LIKE 또는 벡터 검색
- `list_wiki(type?)` — 페이지 목록

도메인 핸들러는 `WikiTools`. JSON-RPC 어댑터는 `openxgram-mcp`가 래핑.

## 사용

```rust
use openxgram_wiki::{WikiFs, WikiTools};
use rusqlite::Connection;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let fs = WikiFs::new("/home/user/.xgram/data/wiki");
    fs.ensure_dirs().await?;

    let conn = Connection::open("/home/user/.xgram/data/xgram.sqlite")?;
    // 0018_wiki_pages.sql 이후 가정.

    let tools = WikiTools::new(&fs, &conn);

    // 새 페이지 작성
    tools.write(
        "entity/alice",
        "# Alice\n\nProfile body.",
        None,
        None,
    ).await?;

    // 검색
    let hits = tools.search("Alice", Some(5))?;
    for h in hits {
        println!("{} — {}", h.id, h.title);
    }

    Ok(())
}
```

## 동기화 (디스크 → DB)

```rust
use openxgram_wiki::Syncer;

let syncer = Syncer::new(&fs, &conn);
let report = syncer.sync_disk_to_db().await?;
println!("added={} updated={} unchanged={} removed={} errors={}",
    report.added, report.updated, report.unchanged, report.removed, report.errors.len());
```

`xgram wiki sync` CLI 명령에 연결.

선택적: `notify` feature 활성 시 백그라운드 watcher.

## 절대 규칙 준수

본 crate는 OpenXgram CLAUDE.md의 6개 절대 규칙을 따른다:

1. **fallback 금지** — content_hash 불일치는 `WikiError`로 raise.
2. **롤백 가능 = 자동 승인** — 본 crate는 디스크 + DB 모두 reversible.
3. **DB 변경 마스터 승인** — 마이그레이션 0018 적용은 마스터 승인 후.
4. **시간대 KST** — Page의 created/updated는 UTC 저장, 표시 시 KST 변환 (호출자 책임).
5. **표 사용 금지** — 문서에서 사용. (코드 내 인라인 OK.)
6. **디스코드 가시성** — CLI 통합 시 작업 보고 (xgram daemon).

## 라이센스

MIT.
