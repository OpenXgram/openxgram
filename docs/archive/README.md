# openxgram — Archived Docs

> **이 폴더의 모든 문서는 ARCHIVED. 정본 아님.**
> 정본 PRD는 워크스페이스 루트의 `docs/PRD-OpenXgram.md` v1.1.

---

## 아카이브 정책

본 폴더는 다음 케이스의 문서를 보관합니다:

1. **이전 PRD 버전** — 정본 v1.1이 superseded
2. **체크리스트·진행 이력** — 정본 PRD §7 Phases에 통합
3. **ADR (Architecture Decision Records)** — historical context, 정본 PRD §9 Resolved Decisions에 통합·갱신
4. **이전 명세** — 정본 PRD에 흡수
5. **유즈케이스** — 정본 PRD §5·§6에 흡수
6. **리서치 자료** — 정본 PRD 작성 토대
7. **handoff (이전 세션 인수인계)** — historical

## 아카이브 일자

2026-05-17 — PRD-OpenXgram v1.1 (Approved) 출시 시점에 일괄 아카이브

## 아카이브된 폴더·파일

| 항목 | 내용 | 현재 위치 (정본) |
|---|---|---|
| `PRD-phase1-5.md` | Phase 1.5 작업 문서 | 정본 PRD §7 (Phase 통합) |
| `CHECKLIST.md` | 작업 체크리스트 | 정본 PRD §7 |
| `architecture/` | 이전 아키텍처 문서 | 정본 PRD §6 |
| `checklists/` | Phase별 체크리스트 | 정본 PRD §7 |
| `decisions/` (ADR) | ADR 6개 | 정본 PRD §9 |
| `prd/` (PRD v1·v2) | 이전 PRD 버전들 | 정본 PRD v1.1 |
| `research/` | Rust crate 서베이 등 | 정본 PRD §3 |
| `specs/` | 메모리·생명주기 명세 | 정본 PRD §3·§4 |
| `usecases/` | 유즈케이스 8개 + README | 정본 PRD §5·§6 |
| `handoff/` | 이전 Claude 세션 인수인계 | historical, 참고만 |

## 참고

여기 문서를 참조할 때는 항상 정본 PRD와 충돌하는지 확인.
충돌 시 **정본 PRD가 우선**.

## 유지되는 문서 (archive 아님)

- `openxgram/README.md` — 빌드·설치·기본 사용. 사용자 진입 문서.
- `openxgram/CHANGELOG.md` — 버전별 변경 이력 (코드 진화 기록)
- `openxgram/CLAUDE.md` — Claude 작업 컨텍스트
- `openxgram/CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md` — 표준 OSS 문서
- `openxgram/LICENSE` — MIT
