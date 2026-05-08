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
- [~] 1.5.1 daemon `/v1/agent/inject` 엔드포인트 추가 (코드 ✓, e2e 검증 보류)
  - [x] 1.5.1.1 route 정의 (`POST /v1/agent/inject`)
  - [x] 1.5.1.2 Bearer auth (mcp_tokens 재사용 — `unauthorized` helper)
  - [x] 1.5.1.3 request body schema (`{sender: String, body: String}`)
  - [x] 1.5.1.4 inbox-from-{sender} 세션 자동 생성 + 메시지 저장
  - [ ] 1.5.1.5 통합 테스트 (`tests/agent_inject.rs`) — 다음 iteration
- [x] 1.5.2 Discord gateway 클라이언트 결정 (HTTP polling, agent 내장)
  - [x] 1.5.2.1 결정: serenity/Python sidecar 둘 다 미채택 → HTTP polling agent 내장 (이유: 새 의존성 0, 단일 binary)
  - [x] 1.5.2.2 의존성: reqwest 재사용 (추가 0)
  - [x] 1.5.2.3 bot token = XGRAM_DISCORD_BOT_TOKEN, channel = XGRAM_DISCORD_CHANNEL_ID
- [x] 1.5.3 Discord 채널 메시지 수신 → 메모리 inject (HTTP API 대신 DB 직접 — 같은 머신이므로)
  - [x] 1.5.3.1 메시지 폴링 (HTTP GET /channels/{id}/messages?after={cursor})
  - [x] 1.5.3.2 sender 매핑 (`discord:{user.id}`)
  - [x] 1.5.3.3 MessageStore.insert + SessionStore.ensure_by_title (피드백 루프 방지: bot 메시지 + sender startswith "discord:" 무시)
- [ ] 1.5.4 검증 (deploy 후 실서버에서)
  - [ ] 1.5.4.1 master Discord 입력 → 5초 안에 inbox 세션 출현
  - [ ] 1.5.4.2 메모리 검색으로 회상 가능

### 1.6 Telegram adapter — ❌
- [ ] 1.6.1 Telegram Bot API 통합
  - [ ] 1.6.1.1 XGRAM_TELEGRAM_BOT_TOKEN, CHAT_ID env — 6단계
  - [ ] 1.6.1.2 long-polling 클라이언트 — 6단계
  - [ ] 1.6.1.3 메시지 수신 → daemon inject — 6단계
- [ ] 1.6.2 Outbound (응답 회신)
  - [ ] 1.6.2.1 sendMessage API 호출 — 6단계
  - [ ] 1.6.2.2 길이 제한 (4096자) trim — 6단계
- [ ] 1.6.3 검증
  - [ ] 1.6.3.1 Telegram 봇과 양방향 라운드트립
  - [ ] 1.6.3.2 Discord + Telegram 동시 활성 동작

### 1.7 메인 에이전트 응답 생성 — ❌
- [ ] 1.7.1 응답 백엔드 trait 추상화 (`ResponseGenerator`)
  - [ ] 1.7.1.1 trait 정의 (`async fn generate(context, input) -> Result<String>`) — 6단계
  - [ ] 1.7.1.2 enum dispatcher (Echo / Anthropic / OpenAgentX) — 6단계
- [x] 1.7.2 EchoGenerator (placeholder)
  - [x] 1.7.2.1 구현 (`"받았습니다: {input}"`)
  - [ ] 1.7.2.2 단위 테스트 — 다음 iteration
- [~] 1.7.3 AnthropicGenerator (기본 동작 ✓, context/비용 로깅 미완)
  - [x] 1.7.3.1 XGRAM_ANTHROPIC_API_KEY env (keystore 옵션은 다음)
  - [x] 1.7.3.2 messages API 호출 (claude-haiku-4-5-20251001 — 빠른 응답)
  - [x] 1.7.3.3 system prompt: "You are {alias}, autonomous AI agent..."
  - [ ] 1.7.3.4 context: 최근 N개 inbox 메시지 동봉 — 다음
  - [ ] 1.7.3.5 토큰 / 비용 로깅 — 다음
- [ ] 1.7.4 검증
  - [ ] 1.7.4.1 master 메시지 → 5초 안에 응답 텍스트
  - [ ] 1.7.4.2 컨텍스트가 누적돼 다음 응답에 반영

### 1.8 서브에이전트 호출 라우팅 — ~ (single-LLM 시뮬레이션 v0)
- [~] 1.8.1 SubInvoker trait 추상화 — single-LLM 멀티 페르소나 (실 sub 라우팅은 다음)
  - [x] 1.8.1.1 system prompt 가 subagents 목록 + 위임 패턴 instruction 포함
  - [ ] 1.8.1.2 enum dispatcher (OpenAgentX / Channel HTTP / Stub) — 다음
- [x] 1.8.2 라우팅 규칙 — LLM 이 단일 호출 안에서 acknowledge/sub/wrap 형식 출력
- [ ] 1.8.3 OpenAgentXInvoker — 다음 (실 sub 노드 도입 시점)
- [ ] 1.8.4 검증 — 실서버 deploy 후 master 가 "@eno 코드 리뷰" 던져 dialogue 형식 확인

### 1.9 응답 회신 라우터 — ❌
- [~] 1.9.1 발신자 종류 판별 (sender prefix)
  - [x] 1.9.1.1 `discord:*` → Discord webhook
  - [ ] 1.9.1.2 `telegram:*` → Telegram sendMessage
  - [ ] 1.9.1.3 `peer:*` → xgram peer_send
- [x] 1.9.2 회신 메모리 기록 (Discord 회신만 우선)
  - [x] 1.9.2.1 outbox-to-{target} 세션 ensure
  - [x] 1.9.2.2 응답 메시지 저장
- [ ] 1.9.3 검증
  - [ ] 1.9.3.1 master 가 보낸 채널에 응답 도착
  - [ ] 1.9.3.2 outbox 세션이 메모리에 정상 저장

### 1.10 conversation 묶음 (메모리) — ❌
- [ ] 1.10.1 schema 마이그레이션
  - [ ] 1.10.1.1 messages 테이블에 conversation_id 추가 — 6단계
  - [ ] 1.10.1.2 마이그레이션 스크립트 작성 — 6단계
  - [ ] 1.10.1.3 기존 메시지에 자동 conversation_id 부여 (NULL → UUID) — 6단계
- [ ] 1.10.2 conversation 추적 로직
  - [ ] 1.10.2.1 inbox 신규 메시지 → 새 conversation_id — 6단계
  - [ ] 1.10.2.2 응답·서브 호출·회신은 같은 conversation 으로 묶음 — 6단계
- [ ] 1.10.3 회상 / 검색 통합
  - [ ] 1.10.3.1 `xgram memory show --conversation {id}` 명령 — 6단계
  - [ ] 1.10.3.2 검증

### 1.11 자율 트리거 — ❌
- [ ] 1.11.1 self-message 메커니즘
  - [ ] 1.11.1.1 ScheduledStore 에 target_kind=self entry 지원 — 6단계
  - [ ] 1.11.1.2 cron entry 발화 시 daemon /v1/agent/inject 호출 — 6단계
- [ ] 1.11.2 기본 cron 등록
  - [ ] 1.11.2.1 매일 09:00 KST "오늘 작업 정리" self-message — 6단계
  - [ ] 1.11.2.2 install.sh 또는 first-run 시 자동 등록 — 6단계
- [ ] 1.11.3 검증
  - [ ] 1.11.3.1 등록된 cron 이 실제 발화하고 처리됨

---

## 2. Phase 2 — 다중 노드 협업

### 2.1 노드 B 부팅 — ❌
- [ ] 2.1.1 다른 서버 install.sh 실행
- [ ] 2.1.2 alias 충돌 검증 (다른 alias 강제)
- [ ] 2.1.3 양쪽 daemon health 확인

### 2.2 A↔B peer 등록 — ❌
- [ ] 2.2.1 A 에서 B peer add (수동)
- [ ] 2.2.2 B 에서 A peer add (수동)
- [ ] 2.2.3 양쪽 list 확인

### 2.3 메시지 라운드트립 — ❌
- [ ] 2.3.1 A → B 한 줄 메시지
- [ ] 2.3.2 B 받음 + 메모리 기록 검증
- [ ] 2.3.3 B 응답 → A
- [ ] 2.3.4 conversation_id 동기 (cross-node)

### 2.4 cross-node 채널 관전 — ❌
- [ ] 2.4.1 양쪽 노드의 forward 가 같은 Discord 채널
- [ ] 2.4.2 A 위임 → B 처리 → 시간순 표시 검증

---

## 3. Phase 3 — 정체성 / 검색

### 3.1 Basenames 핸들 등록 — ❌
- [ ] 3.1.1 `xgram identity claim @<handle>.base.eth`
  - [ ] 3.1.1.1 Basenames registrar 컨트랙트 ABI — 6단계
  - [ ] 3.1.1.2 가스 견적 + 사용자 confirm — 6단계
  - [ ] 3.1.1.3 등록 트랜잭션 — 6단계
- [ ] 3.1.2 manifest 에 핸들 기록
  - [ ] 3.1.2.1 install-manifest.json 확장 — 6단계

### 3.2 ENS text records — ❌
- [ ] 3.2.1 `xgram identity publish`
  - [ ] 3.2.1.1 xgram.handle — 6단계
  - [ ] 3.2.1.2 xgram.daemon — 6단계
  - [ ] 3.2.1.3 xgram.pubkey — 6단계
  - [ ] 3.2.1.4 xgram.bio — 6단계
  - [ ] 3.2.1.5 xgram.visibility — 6단계
- [ ] 3.2.2 visibility=private 시 publish 안 함 검증

### 3.3 핸들 resolver — ❌
- [ ] 3.3.1 `xgram find @<h>` 명령
  - [ ] 3.3.1.1 ENS resolver 호출 — 6단계
  - [ ] 3.3.1.2 records 파싱 — 6단계
  - [ ] 3.3.1.3 캐싱 (TTL 1h) — 6단계

### 3.4 친구 추가 — ❌
- [ ] 3.4.1 `xgram add @<h>` friend request
  - [ ] 3.4.1.1 메시지 magic prefix `xgram-friend-request-v1` — 6단계
  - [ ] 3.4.1.2 daemon 측 pending request 큐 — 6단계
- [ ] 3.4.2 `xgram accept @<h>` / `deny @<h>`
  - [ ] 3.4.2.1 양쪽 peer 자동 등록 — 6단계
  - [ ] 3.4.2.2 거절 시 알림 — 6단계

### 3.5 공개여부 설정 — ❌
- [ ] 3.5.1 `xgram identity set-visibility {mode}`
  - [ ] 3.5.1.1 manifest 갱신 — 6단계
  - [ ] 3.5.1.2 ENS records 동기 — 6단계

---

## 4. Phase 4 — 평판 / Indexer

### 4.1 EAS 어테스테이션 — ❌
- [ ] 4.1.1 EAS schema 정의
  - [ ] 4.1.1.1 xgram-message schema — 6단계
  - [ ] 4.1.1.2 xgram-payment schema — 6단계
  - [ ] 4.1.1.3 xgram-endorsement schema — 6단계
- [ ] 4.1.2 자동 attestation
  - [ ] 4.1.2.1 메시지 발생 시 (옵션) — 6단계
  - [ ] 4.1.2.2 거래 발생 시 — 6단계
- [ ] 4.1.3 가스 정책 (master 부담)

### 4.2 indexer SDK — ❌
- [ ] 4.2.1 crate `openxgram-indexer-sdk`
  - [ ] 4.2.1.1 EAS 이벤트 구독 — 6단계
  - [ ] 4.2.1.2 ENS records 크롤러 — 6단계
  - [ ] 4.2.1.3 랭킹 plugin 인터페이스 — 6단계

### 4.3 첫 indexer 운영 — ❌
- [ ] 4.3.1 docker 이미지 + 호스트
- [ ] 4.3.2 검색 UI (간단 웹)
- [ ] 4.3.3 API: `GET /search?q=...`

### 4.4 사용자 indexer 선택 — ❌
- [ ] 4.4.1 `xgram find --indexer <URL> ...`
- [ ] 4.4.2 manifest default indexer

---

## 5. Phase 5 — 수익화 / 거래

### 5.1 USDC e2e ✅
- [x] 5.1.1 송신자 메모리 기록 (payment_intents)
- [x] 5.1.2 수신자 메모리 기록 (xgr-payment-receipt-v1 magic)
- [x] 5.1.3 onchain 영수증 검증

### 5.2 외부 에이전트 거래 데모 — ❌
- [ ] 5.2.1 시나리오 설계
- [ ] 5.2.2 다른 master 시뮬 노드 가동
- [ ] 5.2.3 USDC 청구 → 결제 → 메모리 + 평판 기록

### 5.3 평판 기반 랭킹 — ❌
- [ ] 5.3.1 거래 이력 점수화
- [ ] 5.3.2 endorsement attestation 반영

---

## 6. 운영 (Phase 0 부수)

### 6.1 systemd unit — ❌
- [ ] 6.1.1 `xgram daemon-install` 가 daemon + agent 둘 다 가동
  - [ ] 6.1.1.1 unit 템플릿 확장 — 6단계
  - [ ] 6.1.1.2 keystore 패스워드 systemd-creds 설계 — 6단계
- [ ] 6.1.2 재부팅 후 자동 시작 검증

### 6.2 업그레이드 흐름 — ❌
- [ ] 6.2.1 install.sh 가 옛 version 감지 → SIGTERM → 재시작
- [ ] 6.2.2 메시지 손실 0 검증

### 6.3 macOS / Windows agent — ❌
- [ ] 6.3.1 CI 매트릭스에 agent 통합 테스트
- [ ] 6.3.2 macOS x86_64 / aarch64 빌드
- [ ] 6.3.3 Windows x86_64 (gateway 모듈 conditional)

### 6.4 Tauri GUI 동기 — ❌ (낮은 우선순위)
- [ ] 6.4.1 agent runtime 통합 또는 분리 결정
- [ ] 6.4.2 핸들 도입 후 재정렬

---

## 7. 최종 재검증 (모든 leaf 완료 후 필수)

각 항목 직접 실행 / 호출 / 시나리오 통과 확인:
- [ ] 7.1 Phase 1 1.5 ~ 1.11 모든 leaf 한 번씩 실제 동작
- [ ] 7.2 Phase 2 e2e 시나리오 1번 (master Discord → A → B → 회신)
- [ ] 7.3 Phase 3 핸들 등록 + resolver + 추가 1번
- [ ] 7.4 Phase 4 검색 1번
- [ ] 7.5 Phase 5 외부 거래 1번
- [ ] 7.6 운영 (재부팅 → 자동 가동 / 업그레이드 / OS 별)

---

## 8. 진행 추적

- 각 leaf 완료 시 6단계 모두 통과 후에만 `[x]`.
- 형식만 만들고 동작 검증 안 한 경우 `[~]` (in-progress) 로 표기.
- 모든 leaf 마쳤다고 끝 X — 7번 재검증 통과해야 100%.

마지막 업데이트: 2026-05-09 23:30 KST
