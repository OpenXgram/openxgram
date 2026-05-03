# PRD → Checklist 변환 — Phase 2 자율 진행

> **생성**: 2026-05-04 03:08 KST
> **기반 PRD**: `docs/prd/PRD-OpenXgram-v2-Phase2.md` (PR #82)
> **시작 상태**: Phase 1 GA v0.1.0 머지 완료. PR #45~#82.
> **완료 기준**: 본 체크리스트 100% [x] + 마스터 정성 검증 통과 → v0.2.0 GA 태깅.

## 진행 규칙 (마스터 지침)

각 leaf 체크리스트 (최하위 4단계 깊이) 는 다음 6 단계를 **순환**:

- **1단계** 중복 코드·기능 구현 여부 검사 + 코드 길이 검사
- **2단계** Context7 로 공식 문서 확인
- **3단계** 코드 구현
- **4단계** 코드 simpler 스킬 (code-simplifier 서브에이전트) 사용
- **5단계** 코드 작동 검증 + 하드코딩 제거 + 정적 요소 제거 확인
- **6단계** 체크리스트 완료 표시 `[ ]` → `[x]`

**중단 없이 진행**. 단순 형식만 만들어 [x] 표시 금지 — 실 작동 검증 필수.

**전체 완료 후 1회 더** 각 항목 직접 실행하며 재검증.

---

## 1. Phase 2.0 — 보안 차단 요인 우선

cross-network 메시징 출시 전 반드시 해결.

### 1.1 PRD-2.0.1 inbound 서명 검증

- [x] 1.1.1 schema 변경 검토 — peer 테이블 column 추가 필요? (이미 public_key_hex 있음 → 불필요)
  - [x] 1.1.1.1 1단계 중복 검사 (peer.public_key_hex 활용 가능 여부)
  - [x] 1.1.1.2 2단계 Context7 (k256 ECDSA verify API)
  - [x] 1.1.1.3 3단계 검토 결과 ADR 메모 (별도 schema 변경 X 결정)
  - [x] 1.1.1.4 4단계 simpler skip (메모만)
  - [x] 1.1.1.5 5단계 결정 reflect 검증
  - [x] 1.1.1.6 6단계 완료 표시
- [x] 1.1.2 daemon process_inbound 에 서명 검증 함수 추가
  - [x] 1.1.2.1 1단계 기존 verify 함수 검사 (keystore::Keypair::verify 활용)
  - [x] 1.1.2.2 2단계 Context7 (k256::ecdsa::VerifyingKey)
  - [x] 1.1.2.3 3단계 verify_envelope_signature(env, peer) 구현
  - [x] 1.1.2.4 4단계 code-simplifier
  - [x] 1.1.2.5 5단계 단위 테스트 통과 + 잘못된 서명 reject
  - [x] 1.1.2.6 6단계 완료 표시
- [x] 1.1.3 검증 실패 envelope drop + WARN 로그
  - [x] 1.1.3.1 1단계 silent error 4 패턴 검토
  - [x] 1.1.3.2 2단계 Context7 (tracing::warn)
  - [x] 1.1.3.3 3단계 process_inbound 분기 추가
  - [x] 1.1.3.4 4단계 code-simplifier
  - [x] 1.1.3.5 5단계 통합 테스트 (위조 envelope 도착 → drop 확인)
  - [x] 1.1.3.6 6단계 완료 표시
- [x] 1.1.4 unknown peer (등록 안 된 from) 처리 — drop vs anonymous-allow
  - [x] 1.1.4.1 1단계 보안 정책 정합성 검토 (마스터 절대 규칙 fallback 금지)
  - [x] 1.1.4.2 2단계 Context7 skip
  - [x] 1.1.4.3 3단계 unknown peer 는 drop + WARN (strict)
  - [x] 1.1.4.4 4단계 simpler skip
  - [x] 1.1.4.5 5단계 통합 테스트
  - [x] 1.1.4.6 6단계 완료 표시

### 1.2 PRD-2.0.2 L0 message 자동 저장

- [x] 1.2.1 envelope → Message 변환 함수
  - [x] 1.2.1.1 1단계 MessageStore::insert API 검토
  - [x] 1.2.1.2 2단계 Context7 skip (자체 코드)
  - [x] 1.2.1.3 3단계 envelope_to_message_insert 작성
  - [x] 1.2.1.4 4단계 simpler
  - [x] 1.2.1.5 5단계 단위 테스트
  - [x] 1.2.1.6 6단계 완료 표시
- [x] 1.2.2 process_inbound 에 통합 (검증 통과 → MessageStore::insert)
  - [x] 1.2.2.1 1단계 중복 검사 (기존 process_inbound 와 통합 위치)
  - [x] 1.2.2.2 2단계 Context7 skip
  - [x] 1.2.2.3 3단계 wiring
  - [x] 1.2.2.4 4단계 simpler
  - [x] 1.2.2.5 5단계 통합 테스트 (peer A 메시지 보냄 → B 의 message store 도착)
  - [x] 1.2.2.6 6단계 완료 표시
- [x] 1.2.3 embedder 호출 통합 (insert 시 임베딩)
  - [x] 1.2.3.1 1단계 default_embedder 활용
  - [x] 1.2.3.2 2단계 Context7 skip
  - [x] 1.2.3.3 3단계 wiring (이미 MessageStore 가 처리)
  - [x] 1.2.3.4 4단계 simpler
  - [x] 1.2.3.5 5단계 임베딩 dim 검증
  - [x] 1.2.3.6 6단계 완료 표시
- [x] 1.2.4 통합 테스트 — peer A → peer B 전체 흐름
  - [x] 1.2.4.1 1단계 기존 peer_send 테스트 재사용 가능 여부
  - [x] 1.2.4.2 2단계 Context7 skip
  - [x] 1.2.4.3 3단계 새 통합 테스트 작성 (sign-verify-store-recall round-trip)
  - [x] 1.2.4.4 4단계 simpler
  - [x] 1.2.4.5 5단계 cargo test 통과
  - [x] 1.2.4.6 6단계 완료 표시

### 1.3 PRD-2.0.3 session 자동 매핑

- [x] 1.3.1 envelope 메타에 session_id 필드 추가
  - [x] 1.3.1.1 1단계 Envelope struct 변경 영향도 검토
  - [x] 1.3.1.2 2단계 Context7 (serde 호환)
  - [x] 1.3.1.3 3단계 transport::Envelope 에 session_id: Option<String> 추가 (backward-compat)
  - [x] 1.3.1.4 4단계 simpler
  - [x] 1.3.1.5 5단계 round-trip 테스트
  - [x] 1.3.1.6 6단계 완료 표시
- [x] 1.3.2 default session 자동 생성 (session_id 없을 때)
  - [x] 1.3.2.1 1단계 SessionStore::ensure_default API 검토
  - [x] 1.3.2.2 2단계 Context7 skip
  - [x] 1.3.2.3 3단계 ensure_default(home_machine) 추가
  - [x] 1.3.2.4 4단계 simpler
  - [x] 1.3.2.5 5단계 단위 테스트
  - [x] 1.3.2.6 6단계 완료 표시
- [x] 1.3.3 alias 별 inbox session — peer alias 가 session_id 로 자동 매핑
  - [x] 1.3.3.1 1단계 session 명명 규칙 (`inbox-from-{alias}`) 정의
  - [x] 1.3.3.2 2단계 Context7 skip
  - [x] 1.3.3.3 3단계 process_inbound 에서 envelope.from → peer alias → session 매핑
  - [x] 1.3.3.4 4단계 simpler
  - [x] 1.3.3.5 5단계 통합 테스트 (3 peer 각각의 inbox session 분리 확인)
  - [x] 1.3.3.6 6단계 완료 표시
- [x] 1.3.4 doctor 통합 — session count 확인
  - [x] 1.3.4.1 1단계 doctor.rs 의 check_memory_layers 활용
  - [x] 1.3.4.2 2단계 Context7 skip
  - [x] 1.3.4.3 3단계 sessions 카운트 추가 (이미 dump 에 있음)
  - [x] 1.3.4.4 4단계 simpler
  - [x] 1.3.4.5 5단계 doctor 출력 확인
  - [x] 1.3.4.6 6단계 완료 표시

### 1.4 PRD-MFA-01 nonce 슬라이딩 윈도우

- [x] 1.4.1 nonce HMAC 생성·검증 함수
  - [x] 1.4.1.1 1단계 hmac crate 의존 검토
  - [x] 1.4.1.2 2단계 Context7 (hmac::Hmac)
  - [x] 1.4.1.3 3단계 generate_nonce / verify_nonce 작성
  - [x] 1.4.1.4 4단계 simpler
  - [x] 1.4.1.5 5단계 단위 테스트 (정상 / 시간 초과 / 재사용)
  - [x] 1.4.1.6 6단계 완료 표시
- [x] 1.4.2 90초 슬라이딩 윈도우
  - [x] 1.4.2.1 1단계 chrono / Instant 선택
  - [x] 1.4.2.2 2단계 Context7 (chrono::Duration)
  - [x] 1.4.2.3 3단계 시간 윈도우 검증
  - [x] 1.4.2.4 4단계 simpler
  - [x] 1.4.2.5 5단계 timing 테스트
  - [x] 1.4.2.6 6단계 완료 표시
- [x] 1.4.3 단일 사용 nonce 캐시 (HashMap<Vec<u8>, Instant>)
  - [x] 1.4.3.1 1단계 메모리 캐시 → DashMap vs HashMap+Mutex
  - [x] 1.4.3.2 2단계 Context7 (dashmap)
  - [x] 1.4.3.3 3단계 NonceCache 구조체
  - [x] 1.4.3.4 4단계 simpler
  - [x] 1.4.3.5 5단계 동시 접근 테스트
  - [x] 1.4.3.6 6단계 완료 표시
- [x] 1.4.4 envelope 통합 — nonce 필드 + 검증 hook
  - [x] 1.4.4.1 1단계 envelope schema 변경 영향
  - [x] 1.4.4.2 2단계 Context7 skip
  - [x] 1.4.4.3 3단계 transport::Envelope 에 nonce 필드 + process_inbound 검증
  - [x] 1.4.4.4 4단계 simpler
  - [x] 1.4.4.5 5단계 replay 시도 → reject 통합 테스트
  - [x] 1.4.4.6 6단계 완료 표시

### 1.5 PRD-2.0.4 rate limit baseline

- [x] 1.5.1 agent 별 호출 카운터
  - [x] 1.5.1.1 1단계 vault_audit 활용 가능?
  - [x] 1.5.1.2 2단계 Context7 skip
  - [x] 1.5.1.3 3단계 in-memory counter (slot per minute)
  - [x] 1.5.1.4 4단계 simpler
  - [x] 1.5.1.5 5단계 단위 테스트
  - [x] 1.5.1.6 6단계 완료 표시
- [x] 1.5.2 config 임계값 (env 또는 config 파일)
  - [x] 1.5.2.1 1단계 하드코딩 금지 — XGRAM_RATE_LIMIT_PER_MIN env
  - [x] 1.5.2.2 2단계 Context7 skip
  - [x] 1.5.2.3 3단계 env 읽기 + default 60
  - [x] 1.5.2.4 4단계 simpler
  - [x] 1.5.2.5 5단계 검증
  - [x] 1.5.2.6 6단계 완료 표시
- [x] 1.5.3 초과 시 429 응답
  - [x] 1.5.3.1 1단계 axum StatusCode 활용
  - [x] 1.5.3.2 2단계 Context7 (axum response builder)
  - [x] 1.5.3.3 3단계 transport handler 통합
  - [x] 1.5.3.4 4단계 simpler
  - [x] 1.5.3.5 5단계 부하 테스트 (60 req → 60 OK / 61번째 429)
  - [x] 1.5.3.6 6단계 완료 표시
- [x] 1.5.4 metrics 노출 — openxgram_rate_limit_rejections_total
  - [x] 1.5.4.1 1단계 기존 metrics provider 활용
  - [x] 1.5.4.2 2단계 Context7 skip
  - [x] 1.5.4.3 3단계 counter 노출
  - [x] 1.5.4.4 4단계 simpler
  - [x] 1.5.4.5 5단계 /v1/metrics 출력 확인
  - [x] 1.5.4.6 6단계 완료 표시

---

## 2. Phase 2.1 — Nostr 메시징 통합

### 2.1 PRD-NOSTR-01 crate 신설

- [x] 2.1.1 openxgram-nostr crate 생성 + Cargo.toml
- [x] 2.1.2 nostr-sdk dep + workspace 등록 + ui/tauri 외부 패턴 검토
- [x] 2.1.3 master keypair → Keys conversion (Keys::parse(secret_hex))
- [x] 2.1.4 단위 테스트 (keys 일관성 — 동일 input 동일 pubkey)

### 2.2 PRD-NOSTR-02 kind 매핑

- [x] 2.2.1 NostrKind enum (L4Trait=30100 / L3Pattern=30200 / L2Memory=30300 / L1Episode=30400 / L0Message=30500 / VaultMeta=30600 / PeerUpdate=30700 / RatchetKey=30050)
- [x] 2.2.2 custom tags schema 정의 (session_id / layer_version / signature)
- [x] 2.2.3 Event builder 함수 per kind
- [x] 2.2.4 deserialize → 5층 메모리 store 매핑

### 2.3 PRD-NOSTR-03 NostrSink

- [x] 2.3.1 NostrSink::publish(kind, content, addressable_id, tags) async
- [x] 2.3.2 multiple relay 동시 publish (nostr-sdk Client send_event_builder)
- [ ] 2.3.3 NIP-44 wrap (옵션 — peer encryption 시) [PRD-NOSTR-05 통합]
- [x] 2.3.4 mock relay 통합 테스트 (publish_to_mock_relay / publish_to_multiple_relays)

### 2.4 PRD-NOSTR-04 NostrSource

- [x] 2.4.1 NostrSource::subscribe(filter) + spawn_listener(callback)
- [ ] 2.4.2 daemon polling task (10s subscribe) [PRD-NOSTR-07 통합]
- [ ] 2.4.3 received event → process_inbound (서명 검증 + L0 저장) [PRD-NOSTR-07 통합]
- [x] 2.4.4 통합 테스트 (mock relay → subscribe → callback)

### 2.5 PRD-NOSTR-05 application-layer ratchet

- [x] 2.5.1 ratchet key 생성·회전 (Ratchet::current/rotate_now, kind 30050 build_announce)
- [x] 2.5.2 메시지 본문 ratchet 키로 wrap (NIP-44 v2 wrap/unwrap)
- [ ] 2.5.3 1주 회전 cron job (scheduler 통합) — Phase 2.4 cron crate 통합 시 처리
- [x] 2.5.4 forward secrecy 회귀 테스트 (sender 옛 secret 폐기 후 unwrap 실패)

### 2.6 PRD-NOSTR-06 self-host relay

- [ ] 2.6.1 nostr-relay-builder dep
- [ ] 2.6.2 `xgram relay serve` 명령 + bind addr (default :7400)
- [ ] 2.6.3 NIP-13 PoW anti-spam
- [ ] 2.6.4 통합 테스트 (자체 relay 띄우기 + publish + subscribe)

### 2.7 PRD-NOSTR-07 peer scheme 인식

- [ ] 2.7.1 peer.address parse — nostr://relay.example.com 인식
- [ ] 2.7.2 peer_send.rs 분기 → nostr route
- [ ] 2.7.3 fallback 정책 (http 우선 → nostr 백업)
- [ ] 2.7.4 통합 테스트 (mixed peer 목록)

### 2.8 PRD-NOSTR-08 NIP-65 relay list

- [ ] 2.8.1 relay list event publish
- [ ] 2.8.2 peer discovery 자동 (= peer-add 자동화)
- [ ] 2.8.3 dedup 로직
- [ ] 2.8.4 통합 테스트

---

## 3. Phase 2.2 — Payment RPC 통합

### 3.1 PRD-PAY-01 alloy + LocalSigner

- [ ] 3.1.1 alloy dep workspace 추가 (alloy = "0.x")
- [ ] 3.1.2 master k256 SigningKey → alloy LocalSigner conversion 함수
- [ ] 3.1.3 nonce 카운터 테이블 (payment_nonce: chain_id, address, next_nonce)
- [ ] 3.1.4 단위 테스트 (서명 결과 secp256k1 일치성)

### 3.2 PRD-PAY-02 sol! IERC20

- [ ] 3.2.1 sol! 매크로로 IERC20::transfer 정의
- [ ] 3.2.2 컴파일타임 ABI 생성 검증
- [ ] 3.2.3 transfer 빌더 헬퍼
- [ ] 3.2.4 erc20 인코딩 결과 비교 (#78 결과와 일치)

### 3.3 PRD-PAY-03 RPC fallback layer

- [ ] 3.3.1 ChainConfig 에 RPC URL 목록 (primary/secondary/tertiary)
- [ ] 3.3.2 tower::ServiceBuilder + RetryLayer 합성
- [ ] 3.3.3 환경변수 override (XGRAM_BASE_RPC_URL 등)
- [ ] 3.3.4 통합 테스트 (primary 실패 → secondary 자동 전환)

### 3.4 PRD-PAY-04 submit() 구현

- [ ] 3.4.1 PaymentStore::submit_via_rpc(intent_id) 시그니처
- [ ] 3.4.2 nonce too low 분기 (재조회)
- [ ] 3.4.3 replacement underpriced 분기 (+15% RBF, 새 attempt)
- [ ] 3.4.4 timeout 분기 + 재시도 정책

### 3.5 PRD-PAY-05 confirmation watcher

- [ ] 3.5.1 watcher tokio task + 1s 폴링
- [ ] 3.5.2 eth_getTransactionReceipt
- [ ] 3.5.3 5블록 soft-confirm → confirmed
- [ ] 3.5.4 64블록 final 마킹

### 3.6 PRD-PAY-06 Reorg 처리

- [ ] 3.6.1 receipt 사라짐 감지
- [ ] 3.6.2 submitted 상태로 회귀
- [ ] 3.6.3 자동 재제출
- [ ] 3.6.4 통합 테스트 (Anvil reorg 시뮬레이션)

### 3.7 PRD-PAY-07 gas oracle

- [ ] 3.7.1 eth_feeHistory 5블록 평균
- [ ] 3.7.2 p50 priority fee
- [ ] 3.7.3 max_fee = 2× base_fee + tip
- [ ] 3.7.4 단위 테스트

### 3.8 PRD-PAY-08 Base testnet 통합 테스트

- [ ] 3.8.1 Anvil fork 또는 Sepolia 환경 셋업
- [ ] 3.8.2 1회 round-trip (draft → signed → submitted → confirmed)
- [ ] 3.8.3 RBF 시나리오
- [ ] 3.8.4 reorg 시뮬레이션

---

## 4. Phase 2.3 — Tauri R/W 확장

### 4.1 PRD-TAURI-01 Vite + Solid + TS 마이그레이션

- [ ] 4.1.1 ui/tauri/web 디렉토리 생성 (이전 frontend 대체)
- [ ] 4.1.2 package.json + vite.config.ts + tsconfig.json
- [ ] 4.1.3 Solid 컴포넌트 — App.tsx + Tabs.tsx
- [ ] 4.1.4 빌드 산출물 검증 (npm run build)

### 4.2 PRD-TAURI-02 plugins 통합

- [ ] 4.2.1 stronghold + dialog + clipboard-manager
- [ ] 4.2.2 notification + updater + store
- [ ] 4.2.3 global-shortcut + single-instance + log
- [ ] 4.2.4 capabilities/*.json 명시 권한

### 4.3 PRD-TAURI-03 Channel API

- [ ] 4.3.1 daemon /metrics 라이브 스트림
- [ ] 4.3.2 pending 큐 변동 push
- [ ] 4.3.3 5초 tick
- [ ] 4.3.4 백프레셔 처리

### 4.4 PRD-TAURI-04 보안 기본

- [ ] 4.4.1 capabilities 윈도우별 분리
- [ ] 4.4.2 CSP default-src 'self'
- [ ] 4.4.3 withGlobalTauri: false
- [ ] 4.4.4 secure storage flow

### 4.5 PRD-TAURI-05 Vault Pending UI

- [ ] 4.5.1 Pending 탭 list 데이터
- [ ] 4.5.2 approve/deny 버튼 + dialog confirm
- [ ] 4.5.3 Stronghold 캐시 master pw
- [ ] 4.5.4 mfa biometric (옵션) 도입

### 4.6 PRD-TAURI-06 Search

- [ ] 4.6.1 검색 박스 + 디바운스
- [ ] 4.6.2 invoke memory_search(query, layers)
- [ ] 4.6.3 가상 리스트 (@tanstack/solid-virtual)
- [ ] 4.6.4 결과 클릭 → detail 페인

### 4.7 PRD-TAURI-07 Pin/unpin memory

- [ ] 4.7.1 우클릭 메뉴 컴포넌트
- [ ] 4.7.2 invoke memory_pin/unpin
- [ ] 4.7.3 UI 즉시 갱신 (optimistic)
- [ ] 4.7.4 에러 처리 + 롤백

### 4.8 PRD-TAURI-08 Episode 강제 종료/시작

- [ ] 4.8.1 Sessions 탭에 컨트롤 버튼
- [ ] 4.8.2 invoke session_reflect
- [ ] 4.8.3 진행률 표시
- [ ] 4.8.4 완료 알림

### 4.9 PRD-TAURI-09 Peer add UI

- [ ] 4.9.1 Peers 탭에 추가 모달
- [ ] 4.9.2 fingerprint 표시 + confirm
- [ ] 4.9.3 alias / address / public_key 입력
- [ ] 4.9.4 검증 (중복 alias / 잘못된 hex)

### 4.10 PRD-TAURI-10 Vault key reveal

- [ ] 4.10.1 reveal toggle 버튼
- [ ] 4.10.2 clipboard-manager 30s auto-clear
- [ ] 4.10.3 화면 30s 마스킹 카운트다운
- [ ] 4.10.4 secure storage flow

### 4.11 PRD-TAURI-11 Payment 한도 변경 UI

- [ ] 4.11.1 Payments 탭에 한도 모달
- [ ] 4.11.2 MFA 재인증 (biometric/master pw)
- [ ] 4.11.3 invoke vault_acl_set
- [ ] 4.11.4 변경 audit 표시

### 4.12 PRD-TAURI-12 자동 업데이트

- [ ] 4.12.1 minisign 키페어 생성 + GitHub Releases endpoint
- [ ] 4.12.2 tauri.conf.json updater 설정
- [ ] 4.12.3 자동 다운로드 + 사용자 confirm
- [ ] 4.12.4 통합 테스트 (mock release)

### 4.13 PRD-TAURI-13 i18n KR/EN

- [ ] 4.13.1 @solid-primitives/i18n dep
- [ ] 4.13.2 ko.json / en.json 메시지 파일
- [ ] 4.13.3 언어 토글 UI
- [ ] 4.13.4 OS locale 자동 감지

### 4.14 PRD-TAURI-14 빌드 산출물 검증

- [ ] 4.14.1 Linux AppImage 빌드 성공
- [ ] 4.14.2 macOS dmg 빌드 (CI matrix 또는 노트)
- [ ] 4.14.3 Windows msi 빌드 (CI matrix 또는 노트)
- [ ] 4.14.4 산출물 크기·서명 검증

---

## 5. Phase 2.4 — 신뢰·감사

### 5.1 PRD-AUDIT-01 hash chain

- [ ] 5.1.1 vault_audit migration — prev_hash / entry_hash / seq
- [ ] 5.1.2 INSERT trigger 또는 코드 — entry_hash 계산
- [ ] 5.1.3 단위 테스트 (chain 무결성)
- [ ] 5.1.4 회귀 테스트 (row 직접 수정 → chain 깨짐 검출)

### 5.2 PRD-AUDIT-02 Merkle checkpoint

- [ ] 5.2.1 audit_checkpoint 테이블 + migration
- [ ] 5.2.2 rs-merkle dep + Merkle root 계산
- [ ] 5.2.3 1시간 cron + ed25519 (또는 master k256) 서명
- [ ] 5.2.4 통합 테스트

### 5.3 PRD-AUDIT-03 verify CLI

- [ ] 5.3.1 `xgram audit verify` 명령 정의
- [ ] 5.3.2 chain 검증 로직
- [ ] 5.3.3 checkpoint 서명 검증
- [ ] 5.3.4 끊긴 지점 보고 + exit code

### 5.4 PRD-AUDIT-04 fault injection 테스트

- [ ] 5.4.1 row 직접 삭제 → verify 깨짐 확인
- [ ] 5.4.2 row 수정 → verify 깨짐 확인
- [ ] 5.4.3 checkpoint 서명 위조 → verify 깨짐 확인
- [ ] 5.4.4 정상 chain → verify 통과

### 5.5 PRD-ROT-01 HD derivation

- [ ] 5.5.1 vault_kek_rotations 테이블 + migration
- [ ] 5.5.2 BIP44 path m/44'/0'/0'/0/N counter 관리
- [ ] 5.5.3 HD child key derive 함수
- [ ] 5.5.4 단위 테스트

### 5.6 PRD-ROT-02 rotate-kek 명령

- [ ] 5.6.1 `xgram vault rotate-kek` CLI
- [ ] 5.6.2 dual-key envelope (old + new)
- [ ] 5.6.3 7일 유예 + 백그라운드 재암호화 잡
- [ ] 5.6.4 진행률 metric (gauge)

### 5.7 PRD-ROT-03 회전 audit

- [ ] 5.7.1 KEK_ROTATE_START / COMMIT / ZEROIZE 이벤트
- [ ] 5.7.2 hash chain 통합
- [ ] 5.7.3 verify 로 회전 검증
- [ ] 5.7.4 통합 테스트

### 5.8 PRD-MFA-02 WebAuthn ADR

- [ ] 5.8.1 passkey-rs 평가
- [ ] 5.8.2 ADR 작성 (도입 여부 결정)
- [ ] 5.8.3 도입 시 prototype
- [ ] 5.8.4 통합 테스트 또는 결정 문서화

---

## 6. Phase 2.5 — 운영성

### 6.1 PRD-OTEL-01 OTLP/HTTP baseline

- [ ] 6.1.1 opentelemetry / opentelemetry_sdk / opentelemetry-otlp deps
- [ ] 6.1.2 tracing-opentelemetry 브릿지
- [ ] 6.1.3 Resource (service.name / version / environment)
- [ ] 6.1.4 통합 테스트 (mock collector)

### 6.2 PRD-OTEL-02 6 함수 instrument

- [ ] 6.2.1 vault.get_as / vault.put
- [ ] 6.2.2 messages.recall_top_k / embedder.encode
- [ ] 6.2.3 payment.sign / payment.broadcast
- [ ] 6.2.4 episode.compact / pattern.classify / transport.send

### 6.3 PRD-OTEL-03 OTel meter

- [ ] 6.3.1 OTel meter exporter 추가
- [ ] 6.3.2 Prometheus pull 병행 (호환)
- [ ] 6.3.3 안정 후 OTel push 일원화
- [ ] 6.3.4 통합 테스트

### 6.4 PRD-OTEL-04 W3C tracecontext

- [ ] 6.4.1 axum/tower 미들웨어 inbound 추출
- [ ] 6.4.2 reqwest 클라이언트 inject
- [ ] 6.4.3 baggage 전파
- [ ] 6.4.4 통합 테스트

### 6.5 PRD-RET-01 retention preview

- [ ] 6.5.1 `xgram retention preview` 명령
- [ ] 6.5.2 layer 별 카운트 (config 임계값)
- [ ] 6.5.3 JSON 출력
- [ ] 6.5.4 단위 테스트

### 6.6 PRD-RET-02 retention apply

- [ ] 6.6.1 `xgram retention apply --layer ...` 명령
- [ ] 6.6.2 dry-run + force
- [ ] 6.6.3 hash chain 기록
- [ ] 6.6.4 통합 테스트

### 6.7 PRD-RET-03 retention cron

- [ ] 6.7.1 scheduler 통합 (03:00 KST)
- [ ] 6.7.2 doctor WARN 정책 위반
- [ ] 6.7.3 metrics 노출
- [ ] 6.7.4 통합 테스트

### 6.8 PRD-RET-04 레이어별 정책

- [ ] 6.8.1 L0 90일 압축 (signature 머클 보존)
- [ ] 6.8.2 L2 unpinned 180일 + LRU
- [ ] 6.8.3 L3/L4 영구
- [ ] 6.8.4 vault_audit 1년 hot + 영구 cold

### 6.9 PRD-BAK-01 age multi-recipient

- [ ] 6.9.1 rage 라이브러리 dep
- [ ] 6.9.2 마스터 X25519 + 비상 복구 2 recipient
- [ ] 6.9.3 backup 명령 옵션 (--age vs --chacha20)
- [ ] 6.9.4 round-trip 테스트

### 6.10 PRD-BAK-02 PQ readiness

- [ ] 6.10.1 wrap 키 인터페이스 추상화
- [ ] 6.10.2 Kyber768 KEM stub
- [ ] 6.10.3 Dilithium 서명 stub
- [ ] 6.10.4 NIST FIPS 203/204 stable 후 교체 계획 ADR

---

## 7. 측정 기준 검증 (전체 완료 후)

### 7.1 정량

- [ ] 7.1.1 v0.2.0 GA 태깅 (Cargo + version.json + CHANGELOG)
- [ ] 7.1.2 신규 통합 테스트 ≥ 80건
- [ ] 7.1.3 workspace clippy 0 warnings
- [ ] 7.1.4 CI ≤ 5분
- [ ] 7.1.5 빌드 캐시 증가 ≤ 25MB

### 7.2 정성 (마스터 검증 필요)

- [ ] 7.2.1 Nostr cross-network round-trip (머신 A → 머신 B 다른 네트워크)
- [ ] 7.2.2 payment intent submit → confirm 1회 (Base testnet)
- [ ] 7.2.3 Tauri 데스크톱 daily-driver
- [ ] 7.2.4 vault rotate-kek round-trip (재암호화 + audit 무결성)
- [ ] 7.2.5 audit verify fault injection 검출
- [ ] 7.2.6 OTel trace Tempo/Jaeger 확인

---

## 8. 최종 검증 (모든 [x] 후 1회 더)

마스터 지침 — "간단한 형식만 만들어 [x] 표시" 방지. 모든 항목 완료 후 다음 절차:

- [ ] 8.1 각 leaf 항목 직접 실행하며 작동 검증
- [ ] 8.2 발견된 미구현 / 부분 구현 → [x] → [ ] 되돌리고 재진행
- [ ] 8.3 모든 통합 테스트 cargo test 한 번 더
- [ ] 8.4 workspace clippy 한 번 더
- [ ] 8.5 cross-network 시나리오 직접 실행 (peer A → B)
- [ ] 8.6 Tauri 앱 daily-driver 직접 사용
- [ ] 8.7 마스터 정성 검증 통과 보고

---

## 9. 진행 로그

각 PR 머지 시 본 섹션에 한 줄 추가:

- (예시) PR #83 — PRD-2.0.1 inbound 서명 검증 — merged 2026-05-04 03:30 KST

(자동 진행 시작 후 채워짐)
