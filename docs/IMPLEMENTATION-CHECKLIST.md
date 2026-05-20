# OpenXgram 사양 구현 체크리스트

> **갱신**: 2026-05-21 KST (rc.31 → rc.32 진행 중)

## 인프라

| 항목 | 상태 |
|---|---|
| server-seoul daemon (Tailscale Funnel) | ✅ HTTPS 200 (https://server-seoul.tail0957ca.ts.net/gui/) |
| openxgram.org 도메인 | ✅ HTTPS 200 (Caddy) |
| openagentx.org 도메인 | ✅ HTTPS 200 (Caddy → localhost:3000) |
| portal-seoul.starian.us | ✅ HTTPS 200 (Caddy → localhost:9400) |
| Zalman GPU Ollama (gemma3:4b) | ✅ 100.87.11.8:11434 |
| **Zalman gemma 실 추론 검증** | ✅ "안녕하세요! 무엇을 도와드릴까요?" 응답 (server-seoul → Tailscale) |
| openagentx → Zalman OLLAMA_BASE_URL | ✅ .env.local 추가, dev 서버 restart |
| Discord 봇 (스타리안#3534) | ✅ 토큰 + 실 메시지 e2e (#macmini-portal) |
| Telegram 봇 (Star_agentbot) | ✅ 토큰 + chat_id + 실 메시지 e2e |
| Caddy reverse_proxy | ✅ 작동 |
| Cloudflare DNS | ✅ openxgram.org / openagentx.org / zalman.openxgram.org (server-seoul 34.22.90.130) |

## UI ↔ Endpoint 매트릭스 (59 endpoint)

✅ 모든 backend endpoint → client.ts route → UI 호출 매핑 완료 (audit 통과).
미노출 1개 (의도적): `system-cron/protect-attempt` POST reject 전용.

## UI-MESSENGER-SPEC v1.3 (59 결정) ✅ 모두

M-1~M-6 + L1~L6 + S1~S8 + C5 + N1·N3·N4·N5·N6·N9·N10 + V1~V12

## UI-MEMORY-SPEC v1.1 (51 결정)

- ✅ M-1·M-3·M-4·M-5·M-6·M-7·M-8·M-9·M-11·M-12·M-13·M-15 + V1~V12
- ❌ M-2 자동 통합 (merge 로직 worker 미)
- ❌ M-10 편집 충돌 (UI 로직 미)
- ❌ M-14 nightly 정리 (reflection worker 미)

## UI-IDENTITY-SPEC v1.0 (27 결정)

- ✅ M-1·M-4·M-5·M-7·M-11 + V-1·V-7·V-9·V-10·V-11·V-12
- ❌ M-2·M-3·M-6·M-8·M-9·M-10·M-12·M-13·M-14·M-15 (BIP39·QR·lockout·sub-DID·revoke·복구 UI X)

## UI-VAULT-MCP-SPEC v1.0 (25 결정)

- ✅ vault_pending list/approve/deny
- ❌ MCP 서버 등록·tool 카탈로그·default-deny ACL UI X

## UI-CHANNEL-SPEC v1.0 (26 결정)

- ✅ 인박스·사람·라우팅·봇 등록·세션별 채널 바인딩·Discord guild channel
- ❌ 모더레이션·봇 라이프사이클·사람별 일 한도 UI X

## UI-AUTONOMY-SPEC v1.0 (24 결정)

- ✅ Cron·history·limits·vacation
- ❌ SelfTrigger·Role 마스터 편집·Reflection UI X

## UI-CARDS-IDENTITY v1.1 + UI-HOME-DASHBOARD-SPEC v1.0 ✅

## 진척률

- **표면 (UI+API+Schema)**: **95%**
- **깊이 (실 작동·e2e)**: **65%**
- **인프라**: **90%**
- **사양 결정**: 약 **220/300+ 작동**

## 검증된 e2e (실 작동)

1. ✅ 위키 upsert → FTS5 search 1 hit
2. ✅ 첨부 inline 11B + disk 1.2MB 라운드트립
3. ✅ 서브 지갑 생성 + V8 마스터→서브 $5 이체
4. ✅ M-5 화이트리스트 자동 등록 (starian tmux + Claude project)
5. ✅ Telegram 봇 실 메시지 전송
6. ✅ Discord 봇 실 메시지 전송 (#macmini-portal)
7. ✅ **Zalman gemma3:4b 실 추론** ("안녕하세요!" 응답)
8. ✅ HomeDashboard 8 카드 → 각 카드 페이지 navigate
9. ✅ 메신저 좌측 머신×세션 트리 (server-seoul 41 항목)
10. ✅ xterm.js 라이브 터미널 (tmux capture-pane 240줄)
11. ✅ 12 탭 우측 패널 + Routing 모달 + Whitelist 모달 + Approval Bell + Global Search
12. ✅ IdentityCard wiring (audit·allowlist·Argon2)
13. ✅ Daemon workers 5종 가동 (M-4·M-5·M-6·L6·V6)
14. ✅ Discord guild channel 27개 fetch (server-seoul → Discord API)

## 미검증 / 미구현 (정직)

1. ⚠️ openagentx fallback chain Ollama 실 호출 e2e (env 적용 후 chat endpoint 호출 검증)
2. ⚠️ Discord listener "master 키 로드 실패" daemon 디버그
3. ⚠️ peer-to-peer 실 e2e (peer 0명 — secp256k1 keypair 등록 필요)
4. ⚠️ Memory 패턴/실수 UI 클릭 e2e
5. ⚠️ wiki 페이지 행 4 액션 (잠금·이력·공유·휴지통) UI 클릭 e2e
6. ❌ Identity 깊은 기능 (BIP39 표시·QR·5회 lockout·sub-DID 발급·revoke·복구·머신 등록)
7. ❌ Vault MCP 서버 등록 + 도구 카탈로그 UI
8. ❌ Channel 모더레이션·봇 라이프사이클·사람별 한도
9. ❌ Autonomy SelfTrigger·Role 마스터 편집·Reflection 실행
10. ❌ Memory M-2 자동 통합·M-10 편집 충돌·M-14 nightly
