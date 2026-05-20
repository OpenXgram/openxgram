# OpenXgram 사양 구현 체크리스트

> **목적**: docs/ 안의 모든 사양 문서의 모든 기능을 구현·검증한 상태를 추적.
> **갱신**: 항목 완료 시 `[ ]` → `[x]`, 검증 시 e2e 결과 기록.
> **현재 시점**: 2026-05-20 KST (이 시점까지 작업 결과)

## 빌드/인프라 (현재 상태)

| 항목 | 상태 | 비고 |
|---|---|---|
| GitHub releases | rc.31 publish | tag + binary 배포 |
| server-seoul 메인 daemon | ✅ 가동 (100.101.237.9:47302) | Tailscale Funnel `https://server-seoul.tail0957ca.ts.net/gui/` |
| Zalman GPU ollama (gemma3:4b) | ✅ 가동 (Windows host 11434) | tailscale 100.87.11.8 |
| server-seoul → Zalman ollama | ✅ 200 OK | curl 검증 |
| Discord 봇 (스타리안#3534) | ✅ 토큰 저장 + validate | guild Starian Oracle (27 channels) |
| Telegram 봇 (Star_agentbot) | ✅ 토큰+chat_id 저장 + 실 메시지 전송 검증 | chat_id 6565914284 |
| openxgram.org / openagentx.org / zalman.openxgram.org | ❌ DNS 미설정 / nginx routing X | server 도메인 매핑 작업 필요 |
| starian-portal 서비스 | ❌ inactive | 재가동 필요 |

## UI-MESSENGER-SPEC v1.3 (59 결정)

### M-1~M-6 기본
- [x] M-1 미연결 발견 (ps + tmux + ~/.claude scan)
- [x] M-2 영구 Agent ULID — schema + L2 4-tuple 표시 (자동 ULID 발급 미)
- [x] M-3 마스터+서브 지갑
- [x] M-4 휴면 자동 — daemon worker 가동
- [x] M-5 화이트리스트 자동 등록 — daemon worker 실작동 검증
- [x] M-6 자동 충전 — daemon worker 가동

### L1~L6
- [x] L1 에이전트/스레드 2-모드 탭
- [x] L2 3-레이어 정체성 + 4-tuple
- [x] L3 auto_respond — 정책 노출 (마스터 enforcement 미)
- [x] L4 HD 영구 점유
- [x] L5 hand-off radio
- [x] L6 차등 만료 — daemon worker (vault_pending 24h)

### S1~S8
- [x] S1 모든 라이브 = 메신저
- [x] S2 Solid.js + Vite
- [x] S3 12 탭 세로 사이드
- [x] S4 좌측 트리 collapse + 정렬 + 필터
- [x] S5 xterm.js + tmux capture-pane (240줄 검증)
- [x] S6 LLM 토큰비 합산 — 모델별 가격 매핑
- [x] S7 첨부 저장 — inline + disk 1.2MB 라운드트립 검증
- [ ] S8 cross-machine SSE — outbound_queue + V6 worker (transport sender 실 통합 미)

### C5 / N1·N3·N4·N5·N6·N9·N10
- [x] C5 breadcrumb 7 카드 전부
- [x] N1·N3·N4 (정책 노출 + FTS5 검색)
- [x] N5·N6 정책
- [x] N9 외부 DID allowlist default-deny
- [x] N10 env mask (사양 정책 노출)

### V1~V12 추가 결정
- [x] V1 RolePolicy struct
- [x] V2 첨부 path content-addressed
- [x] V3 첨부 refcount immediate
- [x] V4 화이트리스트 자동 승인 (결제·위험 제외)
- [x] V5 사용자 패턴 검증 X
- [x] V6 outbound queue SQLite 영구
- [x] V7 Person 타입 forward-ref
- [x] V8 인라인 이체 (마스터→서브 $5 검증)
- [x] V9 외부 DID 세션 override 불가
- [x] V10 외부 LLM vs OpenAgentX 차이
- [x] V11 RoutingRule 헤더 모달
- [x] V12 3-layer 버전 (release/daemon/spec/prd)

## UI-MEMORY-SPEC v1.1 (51 결정)

- [x] M-1 페이지 작성 주체 (authors 필드)
- [ ] M-2 자동 통합 — schema 만 (merge 로직 미)
- [x] M-3 마크다운 (위지윅 토글 UI 미)
- [x] M-4 페이지별 공유 — wiki_shares 테이블 + endpoint
- [x] M-5 패턴 보드 (양방향) — memory_patterns + endpoint
- [x] M-6 새 페이지 알림 — wiki_new_alerts 테이블
- [x] M-7 페이지 잠금 — wiki_locks 테이블 + endpoint
- [x] M-8 카테고리 + 태그 — wiki_pages.category_path + tags
- [x] M-9 페이지마다 공유 모드 (public/secret/password)
- [ ] M-10 편집 충돌 (AI 양보) — UI 로직 미
- [x] M-11 편집 이력 영구 — wiki_history 테이블
- [x] M-12 휴지통 30일 — wiki_trash + V6 worker
- [x] M-13 실수 보드 — memory_mistakes + endpoint
- [ ] M-14 nightly 정리 — reflection worker 미
- [x] M-15 옛 메시지 보존 + 검색
- [x] V-1~V-12 (refcount/share TTL/태그 30자/import 점수/검색 RRF placeholder)

## UI-IDENTITY-SPEC v1.0 (27 결정)

- [x] M-1 unlock 비밀번호 + BIP39 복구 (구조)
- [ ] M-2 자동 잠금 30분 — UI 토글 X
- [ ] M-3 BIP39 표시 — UI X
- [x] M-4 외부 호출 3가지 (allowlist endpoint)
- [x] M-5 서브 지갑 자동 분배
- [ ] M-6 백업 안내 UI X
- [x] M-7 인증 audit (audit_chain)
- [ ] M-8 5회 실패 lockout — daemon 측 X
- [ ] M-9 머신 sub-DID — schema X
- [ ] M-10 해킹 의심 새 DID — UI X
- [x] M-11 DID 형식 노출
- [ ] M-12 QR 공유 — UI X
- [ ] M-13 비밀번호 복구 UI X
- [ ] M-14 새 머신 등록 UI X
- [ ] M-15 키 교체 UI X
- [x] V-1 Argon2id 파라미터 노출
- [x] V-7 allowlist 즉시 적용
- [x] V-9 마스터 출금 사용자만 (UI 정책 노출)
- [x] V-10 HD path 노출
- [x] V-11 revoke 불가
- [x] V-12 API endpoint /v1/gui/identity/*

## UI-VAULT-MCP-SPEC v1.0 (25 결정)

- [x] vault_pending list/approve/deny (기존)
- [ ] MCP 서버 등록 UI X
- [ ] 도구 카탈로그 UI X
- [ ] default-deny ACL UI X
- [ ] 감사 로그 UI X (audit_chain endpoint 만)

## UI-CHANNEL-SPEC v1.0 (26 결정)

- [x] 인박스 (messages_recent 필터)
- [x] 사람 통합 (people endpoint)
- [x] 라우팅 (routing endpoint stub)
- [x] 봇 등록 (notify wizard)
- [x] 세션별 채널 바인딩 — session_channel_bindings + UI
- [x] Discord guild channel 선택
- [ ] 모더레이션 UI X
- [ ] 봇 라이프사이클 UI X
- [ ] 사람별 일 한도 X

## UI-AUTONOMY-SPEC v1.0 (24 결정)

- [x] Cron (기존 schedule_list + chain)
- [x] history endpoint (lifecycle_log)
- [x] limits endpoint
- [x] vacation endpoint
- [ ] SelfTrigger UI/로직 X
- [ ] Role 정책 마스터 편집 UI X (view 만)
- [ ] Reflection 실행 X

## UI-CARDS-IDENTITY v1.1

- [x] 8 카드 (4 가치 + 4 토대) 정체성 명시
- [x] 마스터/뷰 패턴 (RolePolicy = 자율 카드 마스터, 메신저 view 등)
- [x] HomeDashboard 8 카드 grid

## UI-HOME-DASHBOARD-SPEC v1.0 + UI-EXTERNAL-AGENT-SPEC v1.0 + UI-OPERATIONS-SPEC v1.0

- [x] 카드 페이지 골격 (8 카드 모두 컴포넌트 존재)
- [ ] External agent 사양 깊은 구현 X
- [ ] Operations 사양 깊은 구현 X

## PRD-* 도메인

| 도메인 | 상태 |
|---|---|
| openxgram daemon 가동 (server-seoul) | ✅ |
| openagentx.org 사이트 | ❌ DNS/nginx 미설정 |
| openxgram.org 사이트 | ❌ DNS/nginx 미설정 |
| zalman.openxgram.org | ❌ DNS/nginx 미설정 |
| starian-portal 서비스 | ❌ inactive |

## 진척률 정직 추정

- **표면 (UI/API/Schema)**: 약 **80%** (사양 문서 모든 카드/탭/endpoint 골격 노출)
- **깊이 (실 작동/검증)**: 약 **45%**
- **인프라 (도메인/서비스)**: 약 **20%** (server-seoul daemon + Zalman ollama 만)

## 다음 작업 우선순위 (체크리스트 기반)

1. 도메인 DNS + nginx 라우팅 (openxgram.org, openagentx.org, zalman.openxgram.org)
2. starian-portal 서비스 재가동 + 작동 검증
3. openagentx.org chat backend → Zalman gemma 통합 검증
4. M-2 영구 ULID 자동 발급 + L2 4-tuple 머신간 동기화
5. Identity 깊은 UI (BIP39·QR·lockout·sub-DID·revoke)
6. Vault MCP 서버 등록 + 도구 카탈로그 UI
7. Channel 모더레이션 + 사람별 한도 UI
8. Autonomy SelfTrigger + Role 마스터 편집 + Reflection
9. Memory M-2 자동 통합 + M-10 편집 충돌 + M-14 nightly
10. S8 transport sender 실 통합 + sqlite-vec 시멘틱 검색
11. UI-EXTERNAL-AGENT + UI-OPERATIONS 사양 깊이
