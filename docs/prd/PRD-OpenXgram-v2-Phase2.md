# PRD — OpenXgram v0.2 (Phase 2 Cross-Network)

> **버전**: v0.2 spec / Phase 2 일정
> **작성일**: 2026-05-04 KST
> **기반**: Phase 1 GA (v0.1.0, PR #45~#81) — 13 crate Rust 워크스페이스, 5층 메모리, Vault 4단계 보안, MCP stdio+HTTP, Tailscale, Tauri 6 탭, Prometheus
> **승인 절차**: 본 PRD 는 GitHub 조사 (5개 영역 병렬) + 마스터 절대 규칙 (fallback 금지 / DB 변경 승인 / KST / 표 금지 / 디스코드 가시성 / 단순함 / 중복 금지 / 중앙화 / 하드코딩 금지) 준수.

## 0. 결정 요약 (Executive Summary)

각 영역별 GitHub 조사 후 도구·접근법 확정:

- **메시징**: **Nostr (rust-nostr/nostr)** — secp256k1 마스터키 0-friction 통합, custom kind 매핑(L0~L4 별 전용 kind), self-host relay (`nostr-relay-builder`) 가능, NIP-44 E2EE.
- **Payment RPC**: **alloy** (alloy-rs) — k256 호환, sol! 매크로, tower layer retry, Base 1차 + Alchemy 2차 fallback. ethers-rs 는 deprecated.
- **Tauri R/W**: **Vite + Solid.js + TypeScript** + Stronghold + Channel API + 7 공식 plugins. 정적 HTML 폐기.
- **신뢰·감사**: **hash chain + 1시간 Merkle root ed25519 서명** (Sigstore Rekor 패턴 차용, 자체 구현 200~300 줄). HD child key 회전 (BIP44 path index 증가). nonce 슬라이딩 윈도우.
- **운영성**: **OTLP/HTTP-protobuf + tracing-opentelemetry** (gRPC 비추 — 의존 무거움). retention preview/apply 분리. age multi-recipient backup.

각 결정은 우리 5층 메모리·vault·peer·payment 와 결합 시 추가 가치 (예: Nostr addressable kind = L1 episode 자연스러운 namespace).

## 1. 메시징 — Nostr 통합

### 1.1 라이브러리 선택 근거

| 후보 | 인증 | E2EE | 성숙도 | 라이선스 | 우리 호환 | 결정 |
|---|---|---|---|---|---|---|
(표 사용 금지 — 아래 목록으로)

- **Nostr (rust-nostr/nostr, MIT)** — Schnorr secp256k1 (BIP-340), 마스터 키 그대로 사용. NIP-44 ChaCha20+HMAC E2EE. 29 open issues (가장 활발). custom kind 30000~ 자유 정의. ✅ 채택.
- XMTP — MLS E2EE 우수하나 EVM 주소 → MLS Installation key 별도 발급 필요. Nostr 보다 friction 높음.
- Matrix — production 검증 우수하나 MXID + access token 별도 신원. 마스터 키 호환 불가.

### 1.2 Wire-format 매핑 (Nostr kind ↔ OpenXgram 5층)

- `kind: 30100` — L4 traits (addressable, replaceable by `d` tag)
- `kind: 30200` — L3 patterns
- `kind: 30300` — L2 memories
- `kind: 30400` — L1 episodes
- `kind: 30500` — L0 messages (전체 envelope, NIP-44 wrap)
- `kind: 30600` — Vault credential metadata (NIP-44 wrap, 본문은 ACL 화이트리스트 머신만 복호)
- `kind: 30700` — Peer registry update (alias / address / public_key 갱신)

### 1.3 신규 crate: openxgram-nostr

- `nostr-sdk = "0.x"` 의존, master keypair → `Keys::from_secret_bytes`
- `NostrSink::publish(envelope, kind)` / `NostrSource::subscribe(filter, callback)`
- relay URL 은 `peer.address` 의 새 scheme `nostr://relay.example.com` 인식
- daemon 측 inbound polling task (10s subscribe + push)

### 1.4 인프라 주권

- self-host: `nostr-relay-builder` 로 사용자 노드 (= sidecar daemon 의 새 endpoint `:7400`)
- public relay (relay.damus.io 등) fallback — anti-spam 우려는 NIP-13 PoW 로 ratelimit
- NIP-65 relay list — peer discovery 자동 (= peer-add 자동화)

### 1.5 보강: forward secrecy

NIP-44 는 forward secrecy 없음 (스펙 명시). OpenXgram 가 application 레이어에서 ratchet:

- `kind: 30050` — ratchet key broadcast (X3DH-like, 1주 회전)
- 메시지 본문은 ratchet key 로 한 번 더 wrap
- 회전 audit 은 hash chain 에 기록 (§4 참조)

## 2. Payment RPC 통합 — alloy + tower

### 2.1 라이브러리

- **alloy (alloy-rs, MIT/Apache-2.0)**
- ethers-rs 는 공식 deprecated (#2667). 신규 도입 비추.
- revm 은 EVM 실행기, RPC 송신기 아님 — 부적합.

### 2.2 alloy 통합 설계

- `alloy_signer_local::LocalSigner<SigningKey>` ← 우리 마스터 k256 SigningKey 그대로
- `sol! { interface IERC20 { function transfer(address,uint256); } }` — 컴파일타임 ABI
- `RecommendedFillers` (GasFiller + NonceFiller + ChainIdFiller)
- `tower::ServiceBuilder::new().layer(RetryLayer::new(...)).layer(...)` — RPC primary/secondary fallback

### 2.3 Base L2 운영

- Primary RPC: Coinbase Developer Platform (Base 자체 인프라, 무료 rate-limited)
- Secondary RPC: Alchemy (30M CU/월 free)
- Fallback: LlamaRPC (무인증, 백업)
- 가스 oracle: `eth_feeHistory` 5블록 평균 + p50 priority fee
- max_fee = 2× base_fee + tip
- Confirmation: soft-confirm 5블록 (~10초) → `submitted→confirmed`
- Reorg 안전선: 64블록 (~2분) 후 final
- Replacement-by-Fee: 동일 nonce + tip ≥ +12.5% (DOS 룰)

### 2.4 PaymentStore submit() 결합

`draft → signed → submitted → confirmed | failed` 상태 머신:

1. `draft`: PaymentStore 가 `(from, chain_id)` 단위 nonce 카운터 — DB transaction 안에서 get-and-increment
2. `signed`: alloy `TransactionRequest` 빌드 → RLP 서명 → `tx_hash` 결정적 산출 → row 저장 (= idempotency key)
3. `submitted`: `provider.send_raw_transaction(rlp)` — 에러 분류:
   - `nonce too low` → 재조회 후 confirmed/failed 판별
   - `replacement underpriced` → 같은 nonce, +15% tip rebump 후 새 attempt row
   - timeout → row 미진입, draft→signed 재시도 가능
4. `confirmed`: 별도 watcher task `eth_getTransactionReceipt(tx_hash)` 폴링 — blockNumber + 5 ≤ head 도달 시 commit. Reorg 발생 시 `submitted` 회귀.

### 2.5 절대 규칙 정합성

- "fallback 금지" = 조용한 fallback 금지. tower retry layer 는 명시적 로그 남김 → ✅
- "롤백 가능" = idempotent submit, attempt 별 새 row → ✅
- "DB 변경 승인" — payment_intents 테이블은 자기 데이터, 마스터가 명시적 호출 → ✅

## 3. Tauri R/W 확장 — Vite + Solid

### 3.1 Stack 결정

- **정적 HTML 폐기** — R/W 시 폼 검증/모달/가상 리스트 복잡도 폭발
- **Vite + Solid.js + TypeScript** — React 보다 50%+ 작은 번들 (~10KB), signal 기반 = Channel 스트림과 천연 적합
- 가상 스크롤: `@tanstack/solid-virtual`
- 폼 검증: Zod schema
- 다국어 (KR/EN): `@solid-primitives/i18n`

### 3.2 Tauri Plugins (가치 순)

- `tauri-plugin-stronghold` — IOTA Stronghold secure storage. master 패스워드 캐싱·세션 키 보관. Vault L1 캐시.
- `tauri-plugin-dialog` — Pending approve/deny native confirm
- `tauri-plugin-clipboard-manager` — vault_get 복사 후 30초 자동 클리어
- `tauri-plugin-notification` — 새 pending 알림
- `tauri-plugin-updater` — GitHub Releases + minisign 서명
- `tauri-plugin-store` — UI 설정 (탭 위치, 검색 히스토리)
- `tauri-plugin-global-shortcut` — Ctrl+Shift+O quick-open
- `tauri-plugin-single-instance` — 중복 실행 방지

### 3.3 IPC

- 단발 (approve/deny/search): `invoke`
- 라이브 스트림 (pending 큐 변동, /metrics): `tauri::ipc::Channel<T>` (Rust→front, 타입 안전, 백프레셔)

### 3.4 보안

- `capabilities/*.json` 명시 권한 (와일드카드 금지)
- CSP `default-src 'self'; connect-src ipc: http://ipc.localhost`
- `withGlobalTauri: false` — JS 네임스페이스 오염 방지
- vault 평문 절대 invoke 응답 직접 X — Stronghold 에 일시 저장 + 단발 토큰만 webview 에 전달, 자동 zeroize

### 3.5 7 액션 우선순위

1. **Vault Pending approve/deny** — dialog confirm + Stronghold 캐시 master pw, mfa 시 OS biometric/재입력
2. **Search across L0~L4** — 디바운스 입력 → invoke `memory_search(query, layers)` → 가상 리스트
3. **Pin/unpin memory** — 우클릭 메뉴
4. **Episode 강제 종료/시작** — 롤백 가능 → auto 승인
5. **Peer 추가 (machine whitelist)** — confirm 정책, dialog 로 fingerprint 표시 후 승인
6. **Vault key reveal toggle** — clipboard-manager 30s 자동 클리어, 화면 30s 마스킹
7. **Payment 한도 변경** — mfa 정책, biometric/master pw 재인증

## 4. 신뢰·감사 — Hash Chain + Merkle Checkpoint

### 4.1 Audit chain (PRD-AUDIT-01~04)

- **PRD-AUDIT-01**: vault_audit 에 `prev_hash BLOB`, `entry_hash BLOB`, `seq INTEGER` 컬럼. 매 INSERT 시 `entry_hash = SHA256(prev_hash || canonical(row))`.
- **PRD-AUDIT-02**: `audit_checkpoint(seq, merkle_root, signature, signed_at)` 테이블. 1시간마다 ed25519 (또는 master secp256k1) 로 마지막 N 개 entry 의 Merkle root 서명.
- **PRD-AUDIT-03**: `xgram audit verify` CLI — 체인 무결성 + 체크포인트 서명 동시 검증, 끊긴 지점 보고.
- **PRD-AUDIT-04**: 마스터 직접 row 삭제 시도 시 verify 가 즉시 깨짐을 보여주는 회귀 테스트.

라이브러리: `rs-merkle = "1"` (MIT) — 의존 1개, RFC 6962 호환. 자체 구현 200~300 줄.

### 4.2 Secret rotation (PRD-ROT-01~03)

- **PRD-ROT-01**: HD derivation index 관리 — `m/44'/0'/0'/0/N` 의 N 을 rotation 카운터로. `vault_kek_rotations` 테이블 (id, derivation_index, rotated_at, retired_at).
- **PRD-ROT-02**: `xgram vault rotate-kek` 명령 — dual-key envelope. old KEK 7일 유예 (read-only) + 백그라운드 재암호화 잡 (진행률 metric 노출). zero-downtime.
- **PRD-ROT-03**: 회전 audit 이벤트 (KEK_ROTATE_START/COMMIT/ZEROIZE) — hash chain 에 자동 기록.

### 4.3 MFA 강화 (PRD-MFA-01~02)

- **PRD-MFA-01** (Phase 1 보강, 즉시): nonce 캐시 + 슬라이딩 윈도우 — `HMAC(server_secret, agent_id || timestamp || counter)`, 90초 윈도우, 단일 사용 nonce 캐시 (`HashMap<Vec<u8>, Instant>`). 코드 50~80 줄.
- **PRD-MFA-02** (Phase 2): WebAuthn (passkey-rs, MIT/Apache) 통합 ADR — TOTP 대체.

## 5. 운영성 — OTel + Retention + age Backup

### 5.1 OpenTelemetry (PRD-OTEL-01~03)

- Transport: **OTLP/HTTP-protobuf** (gRPC 미사용 — 빌드 +30~40초, 바이너리 +4~6MB. Tempo/Jaeger/Honeycomb 4318 HTTP 모두 지원).
- Crates: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` (HTTP), `tracing-opentelemetry` (브릿지).
- Resource: `service.name=openxgram-core`, `service.version=v0.2.0`, `deployment.environment`.
- Propagator: W3C tracecontext + baggage. axum/tower 미들웨어 inbound 추출, reqwest 클라이언트 inject.

Span 가치:
- vault.get_as / vault.put — 자격증명 latency·실패율
- messages.recall_top_k — 임베딩 + sqlite-vec hot path
- embedder.encode — CPU 병목 후보
- payment.sign / payment.broadcast — 결제 신뢰성
- episode.compact / pattern.classify — 야간 작업 가시화
- transport.send — IPC/Tailscale/XMTP/Nostr 전송 계층 비교

오버헤드: span 당 ~1µs, batch 메모리 ~수MB. 영향 무시 가능.

### 5.2 Retention 정책 (PRD-RET-01~03)

- **PRD-RET-01**: `xgram retention preview [--layer]` — 삭제·압축 대상 카운트만, 변경 X
- **PRD-RET-02**: `xgram retention apply --layer L0 --older-than 90d` — 마스터 승인 후 실행, dry-run 결과 로그, hash chain 에 기록
- **PRD-RET-03**: `xgram retention cron` — 야간 03:00 KST, preview 결과를 `/v1/metrics` 게이지로 노출, doctor 가 정책 위반 시 WARN

레이어별 정책:
- L0 messages: 90일 후 episode summary 압축 (signature 머클루트로 보존), 원본은 cold backup → 삭제
- L2 memories: pinned 무기한 / unpinned 180일 + LRU access_count 가중
- L3 patterns: 영구 (분류기 자체)
- L4 traits: 영구 + 버전 히스토리
- vault_audit: 1년 hot SQLite + 영구 cold (age NDJSON, S3/B2 offload)

### 5.3 Backup 강화 (PRD-BAK-01)

- 현재 ChaCha20-Poly1305 단일 패스워드 → 키 분실 = 영구 손실
- **age multi-recipient** (rage Rust 라이브러리): 마스터 X25519 + 비상 복구용 2 recipient (에이전트 마스터키, 오프라인 종이키)
- PQ readiness: hybrid wrap 인터페이스만 추상화, 실 알고리즘 (Kyber768/Dilithium) 교체는 NIST FIPS 203/204 stable 후

## 6. 절대 규칙 준수 (마스터 지침)

- **fallback 금지** — alloy retry layer 는 명시적 로그. silent fallback 없음.
- **롤백 가능** — payment idempotent submit, vault rotation dual-key 7일 유예, retention preview→apply 분리.
- **DB 변경 마스터 승인** — 모든 새 테이블 (audit_checkpoint, vault_kek_rotations, retention_log) 은 자기 데이터, 마스터 명시 호출.
- **시간대 KST** — 모든 timestamp Asia/Seoul.
- **표 금지** — 본 PRD 도 목록만 사용.
- **디스코드 가시성** — 야간 작업 (reflection, retention, kek rotation) 시작·완료 시 send_message.
- **단순함 1순위** — Nostr (XMTP/Matrix 보다 단순), alloy (ethers 보다 모듈식).
- **중복 금지** — Nostr kind 30000~ 매핑이 envelope 별도 정의 회피.
- **중앙화** — paths/time/env/confirm/ports core hub 유지.
- **하드코딩 금지** — RPC URL, retention threshold, OTel endpoint 모두 config/env. 없으면 default 상수 (단, 단일 위치).

## 7. 작업 분해 (한 줄 PRD 단위)

### Phase 2.0 (보안 차단 요인 우선)

- **PRD-2.0.1** inbound 서명 검증 — daemon process_inbound 에서 envelope.signature_hex 와 peer.public_key_hex 일치 검증 (k256 verify). 실패 시 drop + WARN.
- **PRD-2.0.2** L0 message 자동 저장 — inbound envelope → MessageStore::insert (signature 검증 통과만)
- **PRD-2.0.3** session 자동 매핑 — envelope 의 메타에서 session_id 추출 (없으면 default session 자동 생성)
- **PRD-MFA-01** nonce 슬라이딩 윈도우 + replay 방지

### Phase 2.1 (Nostr 통합)

- **PRD-NOSTR-01** crate 신설 + nostr-sdk + Keys conversion
- **PRD-NOSTR-02** kind 30100~30700 매핑 + custom tags
- **PRD-NOSTR-03** NostrSink::publish + 통합 테스트
- **PRD-NOSTR-04** NostrSource::subscribe + daemon polling task
- **PRD-NOSTR-05** application-layer ratchet (kind 30050)
- **PRD-NOSTR-06** self-host relay (`xgram relay serve`) — nostr-relay-builder
- **PRD-NOSTR-07** xmtp:// peer scheme 인식 → nostr fallback?

### Phase 2.2 (Payment RPC)

- **PRD-PAY-01** alloy dep + LocalSigner conversion + nonce 카운터 테이블
- **PRD-PAY-02** sol! IERC20 + transfer 빌더
- **PRD-PAY-03** tower retry + Coinbase/Alchemy/LlamaRPC fallback
- **PRD-PAY-04** submit() 구현 + 에러 분류 (nonce/replacement/timeout)
- **PRD-PAY-05** confirmation watcher (5블록 soft / 64블록 final)
- **PRD-PAY-06** Replacement-by-Fee (RBF) — +15% tip rebump

### Phase 2.3 (Tauri R/W)

- **PRD-TAURI-01** Vite + Solid.js + TypeScript 마이그레이션
- **PRD-TAURI-02** Stronghold + Channel API + 7 plugins
- **PRD-TAURI-03** Vault Pending approve/deny UI
- **PRD-TAURI-04** Search across L0~L4
- **PRD-TAURI-05** Peer add UI + fingerprint 확인
- **PRD-TAURI-06** Vault reveal + clipboard auto-clear
- **PRD-TAURI-07** Payment 한도 변경 + MFA 재인증
- **PRD-TAURI-08** 자동 업데이트 (minisign)
- **PRD-TAURI-09** i18n (KR/EN)

### Phase 2.4 (신뢰·감사)

- **PRD-AUDIT-01** hash chain (prev_hash + entry_hash + seq)
- **PRD-AUDIT-02** Merkle checkpoint + ed25519
- **PRD-AUDIT-03** xgram audit verify
- **PRD-AUDIT-04** 회귀 테스트 (직접 row 삭제 → verify 깨짐)
- **PRD-ROT-01** HD derivation index 테이블
- **PRD-ROT-02** xgram vault rotate-kek + dual-key 7일 유예
- **PRD-ROT-03** 회전 audit 자동 기록
- **PRD-MFA-02** WebAuthn (passkey-rs) ADR

### Phase 2.5 (운영성)

- **PRD-OTEL-01** OTLP/HTTP exporter + tracing-opentelemetry baseline
- **PRD-OTEL-02** instrument 6 함수 (vault/recall/embed/payment/episode/transport)
- **PRD-OTEL-03** OTel meter exporter (Prometheus pull 병행)
- **PRD-RET-01** retention preview CLI
- **PRD-RET-02** retention apply (dry-run + 실 삭제)
- **PRD-RET-03** retention cron + doctor WARN 통합
- **PRD-BAK-01** age multi-recipient backup

## 8. 측정·완료 기준 (Phase 2 → v0.2 GA)

### 정량
- v0.2.0 GA 태깅
- 신규 통합 테스트 ≥ 80
- workspace clippy 0 warnings
- CI 시간 ≤ 5 분 (file_serial 효과 + 새 crate 추가 부담)
- 의존 트리: alloy + nostr-sdk 추가로 +20MB 빌드 캐시 예상 (수용 가능)

### 정성
- 머신 A → 머신 B (다른 네트워크) Nostr cross-network 메시지 round-trip 성공
- payment intent submit → confirm 1회 성공 (Base testnet)
- Tauri 데스크톱 앱 daily-driver 가능 (Sessions/Vault/Pending/Peers/Payments R/W 모두 GUI)
- vault rotate-kek round-trip (master 키 회전 후 모든 vault 재암호화 + audit chain 무결성 유지)
- audit verify 가 fault injection (DB row 직접 삭제) 즉시 검출

## 9. 병목·성능 고려

- **embedder.encode**: BGE-small ONNX hot path. OTel span 으로 latency 추적 + 캐시 도입 검토
- **sqlite-vec recall_top_k**: large session 시 KNN O(n). cosine prune + index 검토 (sqlite-vec 의 `vec0` virtual table 활용 확장)
- **Nostr relay polling**: 10s interval 적절. event 도달 시 Channel push 로 즉시 반응
- **alloy submit**: tower retry 가 자동 backoff. Base 5블록 confirm = ~10초, watcher 폴링 1s
- **Tauri Channel**: 5초 tick metric push. 폴링 invoke 비추 (배터리·트래픽).
- **/v1/metrics scrape**: 매 호출 DB 12 테이블 COUNT(*) — 1000+ rows 면 비싸짐. counter cache + 변경 시 increment 도입 (post v0.2.1).

## 10. 다음 단계

- 본 PRD 마스터 리뷰
- 승인 후 PRD-2.0.x → PRD-2.1.x → ... 순서로 자율 진행
- 각 PRD 별로 (a) 추가 GitHub 조사 (b) 한 PR 단위 구현 (c) 테스트 (d) 머지
- v0.2.0 GA 태깅 시점은 모든 PRD 완료 + 8 측정 기준 충족 시
