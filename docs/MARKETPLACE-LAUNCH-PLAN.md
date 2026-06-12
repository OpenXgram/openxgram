# OpenXgram 마켓플레이스 런칭 계획 (a~d)

> 상태: 계획 (2026-06-12 작성). 코어 런칭(개인/팀 에이전트 인프라)은 준비 완료(rc.306, seoul+zalman 동기화).
> 마켓 런칭(타 사용자 검색·연결·과금)은 본 계획대로 4갈래 빌드 필요.
> 정본 PRD: `docs/prd/PRD-OpenXgram*.md` §4.4, `docs/prd/PRD-OpenAgentX*.md` §3.3(Phase 2), `docs/prd/PRD-Platform*.md`.

## 전체 흐름
사용자 OpenXgram 에이전트 → OpenAgentX(마켓)에 **게시(publish)** → 타 사용자가 **검색(search)** → A2A/ACP로 **연결(connect)** → **사용(call)** → x402/USDC **결제(pay)** → 게시자 **수익(earn)**.

## 현황 요약 (2026-06-12 감사)
- `openxgram-marketplace` crate(directory·reputation·outbound-calls·policy.rs 한도상수)는 존재하나 **CLI에 미배선 = dead code**. `mcp_serve.rs`에 marketplace 툴 0건.
- GUI `MarketTab.tsx`는 "백엔드 미연결" placeholder. 일일 한도 설정(`payment_set_daily_limit`)만 라이브.
- `invite.rs`: `xgram invite`/`friend accept` 1:1 토큰교환 peer 추가는 작동(검색→연결 플로우는 없음).
- wallet 라우트(`/v1/gui/wallets`·`/topup`·핸들러)·`openxgram-payment` crate(USDC/Base/alloy)는 있으나 UI 미배선.
- OpenAgentX: 하이브리드 검색(`search/hybrid.ts` BM25+pgvector)·`GET /api/v1/agents`·x402/escrow = **Phase 2 설계, 미배포**.
- role-policies = 실행 권한 게이팅(자율도/승인)이지 요금제 게이팅 아님.

## (a) marketplace crate → MCP 툴 배선  [검색·연결의 OpenXgram 끝] — **착수**
- `openxgram-cli/Cargo.toml`에 marketplace 의존성 추가 → `mcp_serve.rs`에 MCP 툴 노출:
  - `marketplace_publish(agent, price, free_quota, visibility)` — (b) 디렉토리에 게시
  - `marketplace_search(query, filters)` — (b) 호출, 타 사용자 공개 에이전트 검색
  - `marketplace_connect(listing_id)` — invite.rs 토큰교환/A2A 재사용해 peer 연결
  - `marketplace_call(agent, task)` — outbound-calls 실행 + policy.rs 한도 검증 + (d) quota 게이트 + (c) 결제
- 핵심 파일: `crates/openxgram-marketplace/*`, `crates/openxgram-cli/src/mcp_serve.rs`, `invite.rs`.
- 디렉토리 백엔드((b)) 미배포 동안: 엔드포인트를 설정값(`XGRAM_MARKETPLACE_URL`)으로, 미설정 시 명시적 "디렉토리 미연결" 에러(가짜 성공 금지).

## (b) OpenAgentX `/api/v1/agents` 디렉토리 연동  [검색의 진실원천]
- OpenAgentX: listing 게시(`POST /api/v1/agents`)·검색(`GET /api/v1/agents?q=`) API 활성화.
- OpenXgram (a)가 호출하는 HTTP 클라이언트 + 에이전트 DID 서명 인증.
- 핵심: `openagentx/.../api/v1/agents`, `search/hybrid.ts`. 의존: pgvector·임베딩.

## (c) wallet/충전/수익 UI 실데이터  [결제 실행]
- MarketTab 지갑/수익 탭 실데이터 배선(잔액·충전·거래내역·수익).
- `marketplace_call` 시 x402/USDC escrow 결제(`payment/escrow.ts` 패턴) + 게시자 정산.
- 핵심: `ui/web/src/components/MarketTab.tsx`, `gui_wallets_*`, `crates/openxgram-payment/*`.

## (d) free-tier 요금제 게이팅  [무료/제한 — 신규]
- 요금제 모델(free quota N회/일 → 초과 시 과금/기능 제한) + 사용량 카운터(per-user·per-agent).
- 게이트: `marketplace_call` 전 quota 확인 → 무료 잔여 통과, 소진 시 (c) 결제 유도.
- 게시자가 가격·무료허용 설정(`marketplace_publish`의 price/free_quota).
- 핵심: 신규 `pricing`/`quota` 모듈 + `policy.rs` 확장.

## 순서 (의존성)
1. (b) 디렉토리 API (검색 진실원천; 미배포면 stub) → 2. (a) crate 배선 → 3. (c) 결제 실행 → 4. (d) 게이팅.
- (a)(c)(d)는 OpenXgram 내부 진행 가능. 최대 미지수 = (b) OpenAgentX(별도 프로젝트·Phase 2).

## 원칙
- 가짜 성공 금지: 디렉토리/결제 미연결 시 placeholder 아닌 **명시적 에러**.
- DB 동적 연결(환경변수/설정), 시크릿 평문 금지(vault), MIT 라이센스 호환.
- 모든 기능 UI에서 검증 가능(검색→연결→사용→결제 e2e).
