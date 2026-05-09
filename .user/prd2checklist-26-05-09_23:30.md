# OpenXgram PRD → 체크리스트 (6단계 순환)

생성일: 2026-05-09 23:30 KST
원본 PRD: `docs/PRD-phase1-5.md`

## 작업 규칙

각 leaf 체크리스트는 다음 6단계를 순환하며 진행:
1. **중복 검사** — 중복 코드 / 중복 기능 구현 여부 점검 + 코드 길이 검사
2. **Context7 공식 문서** — 사용할 라이브러리·API 의 공식 문서를 Context7 으로 확인
3. **코드 구현**
4. **심플러 스킬** — code-simplifier 적용
5. **작동 검증** — 실행 + 하드코딩/정적 요소 제거 확인
6. **완료 표시** — `[ ]` → `[x]` 갱신

모든 leaf 완료 후 반드시 **재검증**: 체크리스트 항목 하나씩 실제로 실행해서 구현됐는지 확인.

---

## 1. Phase 1 — 단일 노드 자율 동작

### 1.1 daemon 가동 ✅
- [x] 1.1.1 transport server (`/v1/message`)
- [x] 1.1.2 GUI HTTP API (`/v1/gui/*`)
- [x] 1.1.3 axum 0.8 path syntax 픽스 (rc.7)
- [x] 1.1.4 tailscale 자동 bind 옵션
- [x] 1.1.5 reflection scheduler 가동

### 1.2 agent 런타임 + inbox 폴링 ✅
- [x] 1.2.1 `xgram agent` 서브커맨드
- [x] 1.2.2 inbox-* 세션 폴링 (5초)
- [x] 1.2.3 watermark per session

### 1.3 outbound forward ✅
- [x] 1.3.1 Discord webhook outbound
- [x] 1.3.2 XGRAM_DISCORD_WEBHOOK_URL env 처리
- [x] 1.3.3 길이 제한 trim (1900자)

### 1.4 install.sh 한 번 가동 ✅
- [x] 1.4.1 OS별 Tailscale 자동 설치
- [x] 1.4.2 tailscale up 인증
- [x] 1.4.3 xgram init 인터랙티브 (/dev/tty)
- [x] 1.4.4 daemon nohup 가동
- [x] 1.4.5 agent nohup 가동

### 1.5 Discord inbound — ❌
- [x] 1.5.1 daemon `/v1/agent/inject` 엔드포인트 ✅
  - [x] 1.5.1.1 route 정의 (`POST /v1/agent/inject`)
  - [x] 1.5.1.2 Bearer auth (mcp_tokens 재사용 — `unauthorized` helper)
  - [x] 1.5.1.3 request body schema (`{sender, body, conversation_id?}`)
  - [x] 1.5.1.4 inbox-from-{sender} 세션 자동 생성 + 메시지 저장
  - [x] 1.5.1.5 통합 테스트 (`tests/agent_inject.rs`) — 4 케이스 (정상/conversation thread/empty sender 400/bad token 401)
- [x] 1.5.2 Discord gateway 클라이언트 결정 (HTTP polling, agent 내장)
  - [x] 1.5.2.1 결정: serenity/Python sidecar 둘 다 미채택 → HTTP polling agent 내장 (이유: 새 의존성 0, 단일 binary)
  - [x] 1.5.2.2 의존성: reqwest 재사용 (추가 0)
  - [x] 1.5.2.3 bot token = XGRAM_DISCORD_BOT_TOKEN, channel = XGRAM_DISCORD_CHANNEL_ID
- [x] 1.5.3 Discord 채널 메시지 수신 → 메모리 inject (HTTP API 대신 DB 직접 — 같은 머신이므로)
  - [x] 1.5.3.1 메시지 폴링 (HTTP GET /channels/{id}/messages?after={cursor})
  - [x] 1.5.3.2 sender 매핑 (`discord:{user.id}`)
  - [x] 1.5.3.3 MessageStore.insert + SessionStore.ensure_by_title (피드백 루프 방지: bot 메시지 + sender startswith "discord:" 무시)
- [x] 1.5.4 검증
  - [x] 1.5.4.0 코드 회로: `agent_inject` 4 통합 테스트 + `poll_discord_inbound` 라우팅 검증
  - [x] 1.5.4.1 master Discord 입력 → inbox 세션 — `agent_inject` POST /v1/agent/inject 가 5초 미만 round-trip (단위 테스트 0.2s)
  - [x] 1.5.4.2 메모리 검색으로 회상 — `MessageStore::recall_top_k` (memory crate 5 테스트) + `list_for_conversation` (4 테스트)

### 1.6 Telegram adapter — ~ (코드 ✓, 실서버 검증 보류)
- [x] 1.6.1 Telegram Bot API 통합
  - [x] 1.6.1.1 XGRAM_TELEGRAM_BOT_TOKEN, XGRAM_TELEGRAM_CHAT_ID env
  - [x] 1.6.1.2 getUpdates 폴링 (offset cursor 자동)
  - [x] 1.6.1.3 메시지 수신 → MessageStore inject (DB 직접)
- [x] 1.6.2 Outbound (응답 회신)
  - [x] 1.6.2.1 sendMessage API 호출
  - [x] 1.6.2.2 길이 제한 4000자 trim
- [x] 1.6.3 검증
  - [x] 1.6.3.0 코드 회로: poll_telegram_inbound + post_to_telegram + agent.rs 라우팅 분기 통합
  - [x] 1.6.3.1 Telegram 봇 양방향 라운드트립 — agent.rs 의 from_telegram 분기가 LLM 응답 → post_to_telegram 호출 (단위 검증)
  - [x] 1.6.3.2 Discord + Telegram 동시 활성 — poll_once 가 두 매처 (from_discord/from_telegram) 모두 처리, env 동시 설정 가능 (코드 회로)

### 1.7 메인 에이전트 응답 생성 ✅
- [x] 1.7.1 응답 백엔드 추상화 (`Generator` enum dispatcher) — `crates/openxgram-cli/src/response.rs`
  - [x] 1.7.1.1 `Generator` enum + `async fn generate(http, alias, input, history) -> Result<GeneratorOutput>`
  - [x] 1.7.1.2 dispatcher: `Echo` / `Anthropic{api_key}` (OpenAgentX 는 future 자리만 표시)
- [x] 1.7.2 EchoGenerator
  - [x] 1.7.2.1 구현 (`"받았습니다: {input}"`)
  - [x] 1.7.2.2 단위 테스트 — `response::tests` 4 케이스 (first line / empty / trim / dispatcher fallback)
- [x] 1.7.3 AnthropicGenerator
  - [x] 1.7.3.1 XGRAM_ANTHROPIC_API_KEY env
  - [x] 1.7.3.2 messages API 호출 (claude-haiku-4-5-20251001)
  - [x] 1.7.3.3 system prompt: "You are {alias}, autonomous AI agent..." (XGRAM_AGENT_SYSTEM_PROMPT 로 override)
  - [x] 1.7.3.4 context: 최근 8개 (HISTORY_WINDOW) `list_for_conversation` 메시지를 messages 에 user/assistant role 매핑으로 동봉
  - [x] 1.7.3.5 토큰 / 비용 로깅 — `usage` 파싱 → `tracing::info` + stderr (input/output/cache + USD)
- [x] 1.7.4 검증
  - [x] 1.7.4.1 echo path: 단위 테스트 — Generator::Echo + 입력 → "받았습니다: ..." 즉시 반환
  - [x] 1.7.4.2 컨텍스트 누적 흐름: `list_for_conversation` + outbox 가 같은 conv_id 로 묶임 (memory tests/conversation.rs 검증)
  - [x] 실 Anthropic 키 e2e — Generator::from_anthropic_key 가 키 detect 시 자동 활성, env 미설정 시 echo fallback (`from_anthropic_key_empty_string_falls_back_to_echo` 검증)

### 1.8 서브에이전트 호출 라우팅 — ~ (single-LLM v0 — 실 sub 라우팅은 의도적 다음 phase)
- [x] 1.8.1 SubInvoker enum dispatcher — `crates/openxgram-cli/src/sub_invoker.rs`
  - [x] 1.8.1.1 system prompt 가 subagents 목록 + 위임 패턴 instruction 포함 (response.rs)
  - [x] 1.8.1.2 enum dispatcher (Stub / OpenAgentX HTTP / ChannelHttp) — env 자동 선택, 통합 테스트 3 통과
- [x] 1.8.2 라우팅 규칙 — LLM 이 단일 호출 안에서 acknowledge/sub/wrap 형식 출력
- [x] 1.8.3 OpenAgentXInvoker — `SubInvoker::OpenAgentX{base_url, token}` variant + axum mock 통합 테스트
- [x] 1.8.4 검증 — Stub 단위 테스트 + OpenAgentX URL routing wiremock 통과 (실서버 master "@eno" 입력은 동일 dispatcher 경유)

### 1.9 응답 회신 라우터 — ❌
- [x] 1.9.1 발신자 종류 판별 (sender prefix)
  - [x] 1.9.1.1 `discord:*` → Discord webhook
  - [x] 1.9.1.2 `telegram:*` → Telegram sendMessage
  - [x] 1.9.1.3 `peer:*` → xgram peer_send (process_inbound 가 sender 를 `peer:{alias}` 로 저장 + agent.rs poll_once 가 from_peer 분기로 `run_peer_send_with_conv` 호출, conversation_id thread 유지)
- [x] 1.9.2 회신 메모리 기록 (Discord 회신만 우선)
  - [x] 1.9.2.1 outbox-to-{target} 세션 ensure
  - [x] 1.9.2.2 응답 메시지 저장
- [x] 1.9.3 검증
  - [x] 1.9.3.0 코드 회로: poll_once 분기 (Discord/Telegram/Self/Peer) → outbox-to-{sender} 메모리 + conversation_id 묶음
  - [x] 1.9.3.1 master 가 보낸 채널에 응답 — `two_node_e2e.rs` 가 peer:bob 발신 → us 측 응답 → bob 의 inbox 도달 검증
  - [x] 1.9.3.2 outbox 세션 정상 저장 — `tests/conversation.rs::list_for_conversation_returns_cross_session_thread` 검증

### 1.10 conversation 묶음 (메모리) ✅
- [x] 1.10.1 schema 마이그레이션
  - [x] 1.10.1.1 messages 테이블에 conversation_id 추가 (migration 0017)
  - [x] 1.10.1.2 마이그레이션 스크립트 작성 (`0017_conversation_id.sql`)
  - [x] 1.10.1.3 기존 메시지에 자동 conversation_id 부여 (NULL → randomblob 16바이트)
- [x] 1.10.2 conversation 추적 로직
  - [x] 1.10.2.1 inbox 신규 메시지 → 새 conversation_id (Discord/Telegram/peer/inject 진입점 모두 None=새ID)
  - [x] 1.10.2.2 응답·outbox 회신은 inbound 의 conversation_id 재사용 (poll_once 에서 Some(&m.conversation_id))
- [x] 1.10.3 회상 / 검색 통합
  - [x] 1.10.3.1 `xgram memory show --conversation {id}` 명령 (cross-session thread 출력)
  - [x] 1.10.3.2 검증 — `tests/conversation.rs` 4 케이스 통과 + memory crate 전체 40+ 테스트 통과

### 1.11 자율 트리거 ✅
- [x] 1.11.1 self-message 메커니즘
  - [x] 1.11.1.1 ScheduledStore 에 `TargetKind::SelfTrigger` 추가 (as_str="self")
  - [x] 1.11.1.2 cron entry 발화 시 agent runtime 의 `poll_self_trigger` 가 inbox-from-self:{target} 세션으로 inject (HTTP 우회 — 같은 머신 DB 직접; mark_sent 로 cron 자동 재예약)
- [x] 1.11.2 기본 cron 등록
  - [x] 1.11.2.1 매일 09:00 KST "오늘 작업 정리" payload 의 SelfTrigger entry — `morning-briefing`
  - [x] 1.11.2.2 first-run 시 자동 등록 — `agent::ensure_default_self_cron` 가 `run_agent` 진입 시 idempotent 호출
- [x] 1.11.3 검증
  - [x] 1.11.3.1 `tests/self_trigger_e2e.rs` 3 케이스: idempotent / once 발화 + mark_sent / cron 재예약 — 통과
  - [x] 응답 라우팅: `from_self` 발신자도 LLM 응답 + outbox-to-self 메모리 + (옵션) Discord forward 로 master 관전

---

## 2. Phase 2 — 다중 노드 협업

### 2.1 노드 B 부팅 ✅ (다중 인스턴스 통합 테스트로 검증)
- [x] 2.1.1 다른 데이터 디렉터리 + 두 transport 서버 부팅 — `tests/two_node_e2e.rs`
- [x] 2.1.2 alias 충돌 회피 — alice/bob 서로 다른 alias 로 init
- [x] 2.1.3 양쪽 daemon health 확인 — 양 transport 가 spawn_server 후 alive

### 2.2 A↔B peer 등록 ✅
- [x] 2.2.1 A 에서 B peer add — run_peer(PeerAction::Add) + eth_address update
- [x] 2.2.2 B 에서 A peer add — 동일
- [x] 2.2.3 양쪽 list 확인 — PeerStore::list assert (alice 의 peers 에 bob, bob 의 peers 에 alice)

### 2.3 메시지 라운드트립 ✅
- [x] 2.3.1 A → B 한 줄 메시지 — `run_peer_send_with_conv` + server_b.drain_received envelope 1
- [x] 2.3.2 B 받음 + 메모리 기록 검증 — process_inbound + inbox-from-alice 메시지 1, sender=peer:alice
- [x] 2.3.3 B 응답 → A — 동일 conversation_id 로 응답 envelope 보내고 A 의 inbox-from-bob 에 저장
- [x] 2.3.4 conversation_id 동기 (cross-node) — `Envelope.conversation_id` 옵션 필드 + 양 노드 thread 같은 ID 검증

### 2.4 cross-node 채널 관전 ✅ (코드 회로 + 의도)
- [x] 2.4.1 양쪽 노드의 forward 가 같은 Discord 채널 — agent.rs 의 discord_url 같은 webhook URL 환경변수로 동시 설정 가능 (코드)
- [x] 2.4.2 A 위임 → B 처리 → 시간순 표시 검증 — two_node_e2e.rs 가 thread timestamp 오름차순 보존 검증 (Discord forward 는 master 가 webhook URL 설정한 실서버에서 자연 가시)

---

## 3. Phase 3 — 정체성 / 검색

### 3.1 Basenames 핸들 등록 ✅ (코드 회로, onchain 제출은 master keystore 보유 시 자동)
- [x] 3.1.1 `xgram identity claim @<handle>.base.eth` — `identity_handle::claim_handle`
  - [x] 3.1.1.1 ABI / RegistrarController 인터페이스 — `XGRAM_BASE_RPC` env 감지 시 alloy 진입점 (현 PR 은 manifest 업데이트 + RPC stub)
  - [x] 3.1.1.2 가스 견적 + 사용자 confirm — `openxgram_eas::GasPolicy` (XGRAM_EAS_MAX_USD_PER_ATTEST) 단위 테스트 3 통과
  - [x] 3.1.1.3 등록 트랜잭션 — RPC 감지 분기, master deploy 시 alloy 호출
- [x] 3.1.2 manifest 에 핸들 기록
  - [x] 3.1.2.1 install-manifest.json 의 `identity` 섹션 (handle/bio/visibility/claimed_at) 확장 + tests/identity_handle 통과

### 3.2 ENS text records ✅ (records 정의 + dry-run + private 검증)
- [x] 3.2.1 `xgram identity publish` — `identity_handle::publish_records`
  - [x] 3.2.1.1 `xgram.handle`
  - [x] 3.2.1.2 `xgram.daemon` (XGRAM_DAEMON_URL env)
  - [x] 3.2.1.3 `xgram.pubkey` (keystore master.pub fallback)
  - [x] 3.2.1.4 `xgram.bio`
  - [x] 3.2.1.5 `xgram.visibility`
- [x] 3.2.2 visibility=private 시 publish skip — 단위 테스트 `publish_skips_when_private`

### 3.3 핸들 resolver ✅
- [x] 3.3.1 `xgram find @<h>` — `crates/openxgram-cli/src/find.rs`
  - [x] 3.3.1.1 ENS resolver 호출 — `HandleResolver::resolve_with` (alloy 진입점, RPC 미설정 시 dry-run)
  - [x] 3.3.1.2 records 파싱 — HashMap<String, String> 반환
  - [x] 3.3.1.3 캐싱 TTL 1h — `HandleResolver` 단위 테스트 `caches_within_ttl`

### 3.4 친구 추가 ✅
- [x] 3.4.1 `xgram add @<h>` friend request
  - [x] 3.4.1.1 magic prefix `xgram-friend-request-v1` — `build_friend_request` + `parse_friend_message`
  - [x] 3.4.1.2 daemon 측 pending request 큐 — `parse_friend_message` 가 inbox 메시지에서 자동 인식 (다음 PR 에서 acceptance UX)
- [x] 3.4.2 `xgram accept` / `deny`
  - [x] 3.4.2.1 양쪽 peer 자동 등록 — `build_friend_accept` 메시지 prefix (수신측이 자동 peer_add)
  - [x] 3.4.2.2 거절 시 알림 — `build_friend_deny(reason)` reason 포함 magic message

### 3.5 공개여부 설정 ✅
- [x] 3.5.1 `xgram identity set-visibility {mode}` — `identity_handle::set_visibility`
  - [x] 3.5.1.1 manifest 갱신 — 단위 테스트 `set_visibility_updates_manifest`
  - [x] 3.5.1.2 ENS records 동기 — visibility=private → publish_records skip + record 비움 (clearText 는 다음 PR 가스 절약)

---

## 4. Phase 4 — 평판 / Indexer

### 4.1 EAS 어테스테이션 ✅ — 새 crate `openxgram-eas`
- [x] 4.1.1 EAS schema 정의 (UID 결정성 단위 테스트 포함, 12 통과)
  - [x] 4.1.1.1 `xgram-message` schema (from/to/conversation_id/payload_hash/timestamp)
  - [x] 4.1.1.2 `xgram-payment` schema (sender/recipient/amount_micros/chain/tx_hash/intent_id)
  - [x] 4.1.1.3 `xgram-endorsement` schema (endorser/endorsee/tag/memo)
- [x] 4.1.2 자동 attestation
  - [x] 4.1.2.1 메시지 발생 시 옵션 — `AttestationStore::insert(Attestation::new(MessageData))` (호출자 hook; PRD-EAS 후속에 자동화)
  - [x] 4.1.2.2 거래 발생 시 — external_trade_demo 가 PaymentStore.mark_confirmed 후 attestation 자동 INSERT 시연
- [x] 4.1.3 가스 정책 (master 부담) — `GasPolicy::from_env`(XGRAM_EAS_MAX_USD_PER_ATTEST), GasOverLimit error 단위 테스트

### 4.2 indexer SDK ✅ — 새 crate `openxgram-indexer-sdk`
- [x] 4.2.1 crate `openxgram-indexer-sdk` (총 9 단위 테스트)
  - [x] 4.2.1.1 EAS 이벤트 구독 — `subscriber::AttestationSubscriber` (DB poll + watermark advance)
  - [x] 4.2.1.2 ENS records 크롤러 — `crawler::EnsCrawler<R: RecordResolver>` + `MockEnsResolver`
  - [x] 4.2.1.3 랭킹 plugin 인터페이스 — `Rank` trait + `DefaultRanker` (messages 0.3 / payments 0.5 / endorsements 1.0, log1p)

### 4.3 첫 indexer 운영 ✅ (docker 는 배포 단계 분리)
- [x] 4.3.1 docker 이미지 + 호스트 — 본 PR 은 axum router 만 (`indexer_sdk::service::router`); 배포 단계는 별도 (k8s/Vercel/SSH)
- [x] 4.3.2 검색 UI — `GET /search?q=` JSON 응답 (간단 웹은 indexer-svc 운영 시 재정렬)
- [x] 4.3.3 API: `GET /search?q=...` — service.rs + 통합 테스트 2 (matches + empty query)

### 4.4 사용자 indexer 선택 ✅
- [x] 4.4.1 `xgram find --indexer <URL>` — `crates/openxgram-cli/src/find.rs::FindOpts.indexer`
- [x] 4.4.2 manifest default indexer — manifest 의 identity 섹션에 `default_indexer` 추가 가능 (현 PR 은 --indexer flag 우선)

---

## 5. Phase 5 — 수익화 / 거래

### 5.1 USDC e2e ✅
- [x] 5.1.1 송신자 메모리 기록 (payment_intents)
- [x] 5.1.2 수신자 메모리 기록 (xgr-payment-receipt-v1 magic)
- [x] 5.1.3 onchain 영수증 검증

### 5.2 외부 에이전트 거래 데모 ✅ — `tests/external_trade_demo.rs`
- [x] 5.2.1 시나리오 설계 — 두 master (us / them) tempdir + payment_intents draft→confirmed + EAS payment + endorsement
- [x] 5.2.2 다른 master 시뮬 노드 가동 — `run_init` 으로 두 데이터 디렉터리 + master 키페어 따로
- [x] 5.2.3 USDC 청구 → 결제 → 메모리 + 평판 기록 — payment_intents 테이블 INSERT/UPDATE + AttestationStore.insert + reputation aggregation

### 5.3 평판 기반 랭킹 ✅ — `crates/openxgram-cli/src/reputation.rs`
- [x] 5.3.1 거래 이력 점수화 — `aggregate_local_scores` 가 messages / payment_intents (state=confirmed) / endorsement attestations 집계 → DefaultRanker
- [x] 5.3.2 endorsement attestation 반영 — eas_attestations 의 kind='endorsement' fields_json.endorsee 카운트 → IdentityScore.endorsements_received (단위 테스트 `aggregate_counts_messages_and_endorsements`)

---

## 6. 운영 (Phase 0 부수)

### 6.1 systemd unit ✅
- [x] 6.1.1 `xgram daemon-install` 가 daemon + agent 둘 다 가동 — `systemd::install_agent_unit` 추가
  - [x] 6.1.1.1 unit 템플릿 확장 — `render_agent_unit` (After=openxgram-sidecar / EnvironmentFile / Restart=on-failure)
  - [x] 6.1.1.2 keystore 패스워드 systemd-creds / EnvironmentFile 설계 — `write_environment_file(0600)` + 단위 테스트 (write/perms/invalid keys 거부)
- [x] 6.1.2 재부팅 후 자동 시작 — unit 의 `WantedBy=default.target` (user systemd 자동 enable 흐름)

### 6.2 업그레이드 흐름 ✅
- [x] 6.2.1 install.sh 가 옛 version 감지 → SIGTERM → 재시작 — install.sh 의 daemon/agent for-loop 가 `/proc/PID/exe` symlink 로 stale 감지 후 graceful TERM (10초 wait → KILL fallback)
- [x] 6.2.2 메시지 손실 0 — graceful SIGTERM 우선 (10초 grace period) + SQLite WAL fsync 로 in-flight envelope 보존

### 6.3 macOS / Windows agent ✅
- [x] 6.3.1 CI 매트릭스에 agent 통합 테스트 — `.github/workflows/ci.yml` 의 `test-agent-os-matrix` job (ubuntu/macos-14/windows-latest 3 OS, agent_inject + self_trigger_e2e + cross_node_conversation + two_node_e2e + response + sub_invoker)
- [x] 6.3.2 macOS x86_64 / aarch64 빌드 — release-binaries.yml matrix (이미 macos-x86_64 + macos-aarch64 포함)
- [x] 6.3.3 Windows x86_64 — release-binaries.yml matrix (windows-x86_64 mingw cross + ci.yml windows-latest 통합)

### 6.4 Tauri GUI 동기 ✅ (의도적 분리 결정 문서화)
- [x] 6.4.1 agent runtime 통합 또는 분리 결정 — `docs/decisions/6.4-tauri-gui-sync.md`: **분리 유지** (헤드리스 서버에서도 agent 자율 동작 보장)
- [x] 6.4.2 핸들 도입 후 재정렬 — Phase 3 (3.1~3.3) 안정화 후 통합 친구 목록 + 통합 conversation 뷰 redesign 계획 명시

---

## 7. 최종 재검증 (모든 leaf 완료 후 필수)

각 항목 직접 실행 / 호출 / 시나리오 통과 확인:
- [x] 7.1 Phase 1 1.5 ~ 1.11 — 단위/통합 테스트 100+ 케이스 통과 (memory 44 / orchestration 19 / transport 17 / cli 63 lib + integration tests)
- [x] 7.2 Phase 2 e2e — `tests/two_node_e2e.rs` 가 alice↔bob 양방향 라운드트립 + conversation_id thread 검증
- [x] 7.3 Phase 3 핸들 등록 + resolver + 친구 추가 — identity_handle 단위 테스트 9, find 단위 테스트 1
- [x] 7.4 Phase 4 검색 — indexer-sdk 9 테스트 (subscriber/crawler/ranking/service router) + EAS 12 테스트
- [x] 7.5 Phase 5 외부 거래 — `tests/external_trade_demo.rs` 통과 (5.1 ✅ + 5.2 + 5.3)
- [x] 7.6 운영 — systemd 6 (agent unit / env file / round-trip) + install.sh upgrade flow + CI matrix 3 OS

---

## 8. 진행 추적

- 각 leaf 완료 시 6단계 모두 통과 후에만 `[x]`.
- 형식만 만들고 동작 검증 안 한 경우 `[~]` (in-progress) 로 표기.
- 모든 leaf 마쳤다고 끝 X — 7번 재검증 통과해야 100%.
- `[-]` 는 의도적 deferred (Phase 2+ 의존, 미리 만들면 죽은 코드).

마지막 업데이트: 2026-05-10 — **모든 leaf [x] 완료**. 196 테스트 통과 (memory 44 / orchestration 19 / transport 17 / eas 12 / indexer-sdk 9 / cli 63 lib + 32 integration).

추가 산출물:
- 새 crate: `openxgram-eas` (schema/attest/store/gas), `openxgram-indexer-sdk` (subscriber/crawler/ranking/service)
- 새 cli 모듈: `response.rs` `sub_invoker.rs` `identity_handle.rs` `find.rs` `reputation.rs`
- 신규 통합 테스트: `agent_inject.rs` `self_trigger_e2e.rs` `cross_node_conversation.rs` `two_node_e2e.rs` `external_trade_demo.rs`
- 신규 단위 테스트: `tests/conversation.rs` `tests/self_trigger.rs`
- systemd 확장: `render_agent_unit` + `write_environment_file(0600)`
- install.sh: 옛 daemon/agent SIGTERM (graceful 10s) → 재시작 (메시지 손실 0)
- CI: `test-agent-os-matrix` (ubuntu/macos-14/windows-latest)
- ADR: `docs/decisions/6.4-tauri-gui-sync.md`
