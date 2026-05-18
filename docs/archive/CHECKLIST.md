# OpenXgram 구현 체크리스트

PRD: `docs/PRD-phase1-5.md` 의 모든 요구사항을 실행 가능한 작업으로 분해.

---

## Phase 1 — 단일 노드 자율 동작

### F1.1 daemon 가동 ✅
- [x] xgram daemon 구현 (transport + scheduler + GUI HTTP API)
- [x] axum 0.8 path syntax 픽스 (rc.7)
- [x] tailscale 자동 bind 옵션
- [x] /v1/gui/health, /v1/gui/status 엔드포인트

### F1.2 agent 런타임 + inbox 폴링 ✅
- [x] `xgram agent` 서브커맨드
- [x] inbox-* 세션 폴링 (5초 주기)
- [x] watermark per session (~/.openxgram/agent-state.json)

### F1.3 outbound forward (관전) ✅
- [x] Discord webhook outbound
- [x] XGRAM_DISCORD_WEBHOOK_URL env

### F1.4 install.sh 한 번 가동 ✅
- [x] OS별 Tailscale 자동 설치
- [x] tailscale up 인증 안내
- [x] xgram init (alias + keystore 패스워드 /dev/tty 인터랙티브)
- [x] daemon nohup 가동
- [x] agent nohup 가동
- [x] 페어링 URL 출력

### F1.5 Discord inbound — ❌
- [ ] Discord bot 토큰 입력 흐름 (install.sh prompt 또는 env)
- [ ] Discord gateway 클라이언트 구현
  - 옵션 A: Rust serenity/twilight crate 내장 (의존성 무거움)
  - 옵션 B: Python sidecar 스크립트 (간단, 별 process)
  - 결정 필요
- [ ] 채널 메시지 수신 → daemon /v1/agent/inject 호출
- [ ] daemon 에 /v1/agent/inject 엔드포인트 추가 (Bearer auth)
- [ ] inject → inbox-from-discord-master 세션에 저장
- [ ] 검증: master Discord 입력 → 5초 안에 inbox 세션 출현

### F1.6 Telegram adapter — ❌
- [ ] XGRAM_TELEGRAM_BOT_TOKEN + CHAT_ID env
- [ ] Telegram Bot API long-polling 구현 (또는 webhook 모드)
- [ ] 인바운드 → /v1/agent/inject
- [ ] 아웃바운드 → sendMessage API
- [ ] 검증: Discord 와 동일 양방향 동작

### F1.7 메인 에이전트 응답 생성 — ❌
- [ ] 응답 백엔드 추상화 (trait `ResponseGenerator`)
- [ ] 구현 1: EchoGenerator (placeholder)
- [ ] 구현 2: AnthropicGenerator (XGRAM_ANTHROPIC_API_KEY env)
  - [ ] reqwest 로 messages API 호출
  - [ ] system prompt: "You are {alias}, …"
  - [ ] context: 최근 N개 inbox 메시지
- [ ] (다음) 구현 3: OpenAgentXGenerator
- [ ] 검증: master 메시지 → 5초 안에 응답 텍스트 생성

### F1.8 서브에이전트 호출 라우팅 — ❌
- [ ] 라우팅 규칙 — 응답 텍스트 안의 "@eno {task}" 패턴 detection
- [ ] 서브 호출 백엔드 추상화 (trait `SubInvoker`)
- [ ] 구현 1: OpenAgentXInvoker (HTTP API 호출)
- [ ] 구현 2: ChannelHttpInvoker (Starian Channel HTTP bridge — 있다면)
- [ ] 비동기 응답 수신 + timeout
- [ ] 검증: "@eno 코드 리뷰해" → eno 응답 받기

### F1.9 응답 회신 — ❌
- [ ] 응답 송출 라우터: 발신자 종류에 따라 경로 분기
  - discord:* → Discord webhook post
  - telegram:* → Telegram sendMessage
  - peer:* → xgram peer_send
- [ ] 메모리 기록 (outbox-to-{target} 세션)
- [ ] 검증: 응답이 master 가 보낸 채널에 다시 나타남

### F1.10 메인 활동 메모리 기록 — ❌
- [ ] conversation_id 필드 도입 (messages 테이블 마이그레이션)
- [ ] inbox / outbox / delegation 세션이 같은 conversation 으로 묶임
- [ ] 메모리 검색 시 conversation 단위로 묶어 보여주기
- [ ] 검증: 한 대화 (master 질문 → 메인 응답 → 서브 호출 → 회신) 가 메모리에서 한 conversation 으로 회상됨

### F1.11 자율 트리거 — ❌
- [ ] openxgram-scheduler 의 cron 항목으로 self-message 등록
- [ ] schedule entry: target=self, payload="작업 정리 보고", cron="0 9 * * *"
- [ ] agent 가 self-message 받으면 평소 흐름과 동일하게 처리
- [ ] 검증: 매일 09:00 KST 에 메인이 자율 메시지 생성 → Discord 에 보고

---

## Phase 2 — 다중 노드 협업

### F2.1 노드 B 부팅 — ❌
- [ ] 다른 서버에서 install.sh 실행 → 두 번째 노드 가동
- [ ] alias 충돌 방지 검증

### F2.2 A↔B peer 등록 — ❌
- [ ] A 에서 `xgram peer add <B alias> <B address> <B daemon URL>`
- [ ] B 에서 동일하게 A 등록
- [ ] (Phase 3 의 핸들 resolver 도입 전엔 수동 OK)

### F2.3 메시지 라운드트립 — ❌
- [ ] A → B 한 줄 메시지 전송
- [ ] B 가 받음 + 메모리 기록
- [ ] B 응답 → A 가 받음 + 메모리 기록
- [ ] 양쪽 outbox/inbox 세션 묶임 (conversation_id)

### F2.4 cross-node 흐름 채널 관전 — ❌
- [ ] master 가 한 Discord 채널에서 A 와 B 의 대화를 모두 관전
- [ ] 각 노드의 forward 가 같은 채널을 share
- [ ] 검증: A 메인이 B 에 위임 → B 처리 → 양쪽 forward 가 채널에 시간순 표시

---

## Phase 3 — 정체성 / 검색

### F3.1 Basenames 핸들 등록 — ❌
- [ ] `xgram identity claim @<handle>.base.eth`
- [ ] alloy 로 Basenames registrar 호출
- [ ] 가스 견적 + 사용자 확인 prompt
- [ ] 등록 성공 후 manifest 에 핸들 기록

### F3.2 ENS text records publish — ❌
- [ ] `xgram identity publish` — text records:
  - [ ] xgram.handle
  - [ ] xgram.daemon (URL)
  - [ ] xgram.pubkey
  - [ ] xgram.bio
  - [ ] xgram.visibility (public|unlisted)
- [ ] private 모드에서는 publish 안 함

### F3.3 핸들 resolver — ❌
- [ ] `xgram find @<handle>` — onchain ENS resolver 호출
- [ ] xgram.* records 파싱
- [ ] 결과 캐싱 (TTL)

### F3.4 친구 추가 플로우 — ❌
- [ ] `xgram add @<handle>` — friend request 메시지 전송
- [ ] 받는 쪽: pending request 큐 (vault 활용)
- [ ] `xgram accept @<handle>` / `xgram deny @<handle>`
- [ ] 양쪽 peer 자동 등록

### F3.5 공개여부 — ❌
- [ ] `xgram identity set-visibility public|unlisted|private`
- [ ] private: ENS 등록 자동 제거
- [ ] unlisted: ENS 등록은 유지하되 indexer 가 인덱싱 skip

---

## Phase 4 — 평판 / Indexer

### F4.1 EAS 어테스테이션 — ❌
- [ ] EAS schema 정의 (xgram-message, xgram-payment, xgram-endorsement)
- [ ] 메시지/거래 발생 시 attest (옵션, master 설정으로 활성화)
- [ ] 가스비 부담 정책

### F4.2 indexer SDK — ❌
- [ ] Rust crate `openxgram-indexer-sdk`
- [ ] EAS 이벤트 구독 + ENS records 크롤링
- [ ] 랭킹 알고리즘 plugin 인터페이스

### F4.3 첫 indexer (openxgram.org/search) — ❌
- [ ] 도커 이미지 + 운영 호스트
- [ ] 검색 UI (간단 웹)
- [ ] API: `GET /search?q=...&limit=...`

### F4.4 사용자 indexer 선택 — ❌
- [ ] `xgram find --indexer <URL> @keyword`
- [ ] manifest 에 default indexer 저장

---

## Phase 5 — 수익화 / 거래

### F5.1 USDC e2e ✅
- [x] 송신자 메모리 기록 (payment_intents)
- [x] 수신자 메모리 기록 (xgr-payment-receipt-v1 magic)
- [x] onchain 영수증 검증

### F5.2 외부 에이전트 거래 데모 — ❌
- [ ] 다른 master 의 에이전트 (실제 또는 시뮬레이션) 와 거래 시나리오
- [ ] 작업 수행 → USDC 청구 → 결제 → 메모리 + 평판 기록

### F5.3 평판 기반 랭킹 — ❌
- [ ] Phase 4 indexer 가 거래 이력 점수화
- [ ] 후기 / endorsement attestation 반영

---

## 운영 (Phase 0 부수)

### OP1 systemd unit — ❌
- [ ] `xgram daemon-install` 가 daemon + agent 둘 다 가동하는 unit 생성
- [ ] keystore 패스워드 — systemd-creds 또는 EnvironmentFile 보안 권장사항 문서화
- [ ] 재부팅 후 자동 시작 검증

### OP2 업그레이드 흐름 — ❌
- [ ] install.sh 가 옛 daemon 버전 감지 → 자동 SIGTERM → 새 binary 재시작
- [ ] agent 동일 처리
- [ ] 메시지 손실 0 검증

### OP3 macOS / Windows agent 검증 — ❌
- [ ] CI 매트릭스에 agent 통합 테스트
- [ ] macOS x86_64 / aarch64 빌드 검증
- [ ] Windows x86_64 (gateway 모듈 conditional compile)

### OP4 Tauri GUI 동기 — ❌ (낮은 우선순위)
- [ ] 데스크탑 GUI 의 agent runtime 통합 (또는 분리 결정)
- [ ] Phase 3 핸들 도입 후 재정렬

---

## 즉각 다음 (이번 / 다음 세션)

1. **F1.5 ~ F1.10 한 묶음 PR**
   - daemon /v1/agent/inject 엔드포인트
   - agent 응답 생성 (Echo → Anthropic)
   - 서브 호출 (OpenAgentX adapter)
   - 응답 회신 라우터
   - conversation_id 마이그레이션
2. **F1.11 별 PR** — scheduler 통합
3. **OP1** — systemd unit 마무리

---

## 진행 추적

각 항목 완료 시 `[ ]` → `[x]`. 부분 완료는 sub-bullet 으로 분해. PR 머지마다 이 파일 업데이트.

마지막 업데이트: 2026-05-09 KST
