# 통합 현황 그리드 — 구현 계획 (갭 클로징)

> 정본 목표: `.handoff/KAKAOTALK-MESSENGER-SPEC.md` (마스터 확정 2026-06-25)
> 이 문서: 그 목표 대비 **현재 코드 갭 + 채우는 순서**. 마스터 승인(2026-06-25): **A 즉시 + B/C 승인**.
> 협업: `aoe_starianset-hermes_a6d5031f` (검증/협력). 검증은 Claude 자가검증 금지 → 다른 LLM(Codex/Gemini/hermes).

## 현재 상태 (4-에이전트 코드 탐색 종합, 2026-06-25)
코드베이스가 목표의 ~70% 구현됨. 빠진 것은 좁고 명확.

### 이미 구현됨 (재사용)
- 그리드 렌더: `ui/web/src/components/KakaoShell.tsx` (#1434–1547, `.dash` 블록)
- 컬럼 PRESENT: 순번·상태·이름·역할·세션id·PATH(폴더)·머신·종류·정본주소·액션
- 액션 PRESENT: 새창·종료·재시작·spawn·등록·삭제 + 인라인편집(이름/역할)
- 백엔드 roster: `crates/openxgram-cli/src/daemon_gui.rs` `gui_roster()` → `RosterEntryDto[]`
  - **이미 계산해서 내려줌**: wallet_balance·income(earned)·expense(spent)·status·is_peer/has_agent(등록상태)·rating/review_count(external)
- 등록 3방법 전부 존재: tmux CLI(`register_subagent` mcp_serve.rs:1886) / GUI 버튼(`gui_agents_register` daemon_gui.rs:5671) / GUI 직접등록
- 친구 시스템: `gui_friends_roster` + policy CRUD (daemon_gui.rs:5431+)
- tmux 전수 열거: `detect_tmux()` daemon_gui_sessions.rs:285 (등록·미등록·cwd·machine·session_id)
- ACP spawn/close: openxgram-acp/src/mcp.rs (spawn:74, close:99)
- 지갑: sub_wallets(0021)+wallet_ledger(0055), topup 엔드포인트 `/v1/gui/wallets/topup`

## 갭 → 채우는 순서

### Phase A — 프론트 배선만 (DB 무관, UI 즉시 검증) ★착수
- KakaoShell 그리드에 컬럼 추가: 등록상태·지갑잔액·수입·지출·별점·평가 (RosterEntryDto에 이미 존재)
- 액션 추가: 금액추가(topup 존재)·금액이전(엔드포인트 확인 → 없으면 C로)
- 선결: Rust `RosterEntryDto` 직렬화 필드명 ↔ 프론트 `GridRow`/`RosterEntryDto` TS 타입 대조
- 검증: headless 렌더(마운트 크래시 방지) → 배포 전 필수

### Phase B — 마이그레이션 3개 (승인 완료)
- 0067: `agent_profiles.token_price_per_million` (REAL) + PATCH `/v1/gui/agents/{alias}/token-price`
- 0068: 샘플 — `agent_profiles.sample_text` + `sample_files`(JSON) 또는 landing_url + SET 엔드포인트
- 0069: 인지도 metrics 테이블(views·calls·rank) + 내부 에이전트 별점/평가 (external_reputation 확장 or 신규)
- 규칙: .sql 파일 + migrate.rs 배열 등록 **둘 다** (openxgram-migration-registration)
- roster JOIN 확장 → 새 필드 RosterEntryDto에 추가 → 프론트 컬럼

### Phase C — 백엔드 로직 보강
- 금액이전: agent sub_wallet → 메인지갑 transfer 엔드포인트
- ACP 원자적 재시작 (현재 kill+spawn만)
- ACP 대화방 ↔ 동일 PATH tmux 상관 엔드포인트 (cwd 매칭) — 스펙 §4
- 친구 즐겨찾기/상단고정 (현재 프론트 UX state만, DB 개념 없음)

## 검증 원칙
- 매 페이즈: 빌드 실제 exit code + headless 렌더 + UI 변화 확인 (규칙 #8)
- 최종 검증은 다른 LLM/hermes 위임 (Claude 자가검증 금지)
- 버전 bump + push (work-completion-rules)
