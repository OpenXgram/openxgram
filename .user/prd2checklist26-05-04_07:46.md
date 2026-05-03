# OpenXgram Phase 2 잔여 PRD 체크리스트 (v0.2.0 GA 까지)

> 생성: 2026-05-04 07:46 KST
> 기반 PRD: docs/prd/PRD-OpenXgram-v2-Phase2.md
> 대상: Phase 2.2 Payment / 2.3 Tauri / 2.4 Trust·Audit / 2.5 Observability + Phase 2.1 deferred 5건
> 완료 기준: 모든 [x] + 최종 직접실행 재검증 + clippy 0 warnings + workspace 빌드 통과

## 6단계 사이클 (각 leaf 마다 순환 — 절대 준수)

각 최하위 체크리스트(level 4) 는 다음 순서로 진행하고, 6단계 완료 시점에만 [x] 표시한다.
실제 구현 없이 형식만 만든 경우 [x] 금지.

1. 중복 검사 — 기존 코드 grep, 동일 기능·중복 함수·과도한 길이 점검
2. Context7 공식 문서 — 사용 라이브러리 API 정확성 확인
3. 코드 구현 — 실제 동작 가능한 코드, 스텁 금지
4. simpler 스킬 — 응집도·중복·하드코딩·정적 요소 제거
5. 작동 검증 — cargo test / 직접 실행 / 결과 확인
6. 체크리스트 [x] — 6단계 모두 통과 시에만

---

## Phase 2.1 잔여 (Nostr deferred 5건)

### [ ] 1. PRD-NOSTR-09 NIP-44 peer 암호화 통합 (deferred 2.3.3)

#### [ ] 1.1 Sender 측 NIP-44 wrap 통합

##### [x] 1.1.1 peer_send.rs 에서 envelope → NIP-44 ciphertext 래핑

  - [x] 1단계 중복검사: ratchet.rs nip44::encrypt 사용 중, peer_send.rs 는 plaintext publish — 추가 필요
  - [x] 2단계 Context7: nostr::nips::nip44::{encrypt, decrypt, Version::V2} API 확인 (ratchet.rs 검증된 시그니처)
  - [x] 3단계 구현: openxgram-nostr 에 encrypt_for_peer/decrypt_from_peer 헬퍼 + send_via_nostr 에서 envelope JSON 을 NIP-44 wrap → ciphertext publish
  - [x] 4단계 simpler: V2 캡슐화, empty plaintext raise, sink/sender_keys 단일 분기
  - [x] 5단계 검증: send_via_nostr_publishes_to_mock_relay (NIP-44 라운드트립 추가) + nostr 17 lib tests pass, clippy 0
  - [x] 6단계 [x]

##### [x] 1.1.2 publish 시 ciphertext 만 본문으로, p-tag 유지

  - [x] 1단계 중복검사: sink.publish 호출처 — peer_send 1곳 + ratchet announce 1곳
  - [x] 2단계 Context7: Event.content = ciphertext (NIP-44 base64), NIP-33 addressable kind 30000~ 은 d-tag 필수 (MockRelay 가 검증)
  - [x] 3단계 구현: send_via_nostr 가 ciphertext 만 본문, p-tag + d-tag(envelope.nonce/timestamp) 포함
  - [x] 4단계 simpler: d-tag 자동 산출 (nonce → ts fallback) 단일 분기
  - [x] 5단계 검증: published_event_carries_ciphertext_and_p_tag — MockRelay 수신 후 content ≠ plaintext + p-tag 보존 검증
  - [x] 6단계 [x]

##### [x] 1.1.3 ratchet wrap 와 NIP-44 wrap 의 순서 결정

  - [x] 1단계 중복검사: ratchet.wrap 와 encrypt_for_peer 둘 다 NIP-44 v2 — 이중 wrap 시 보안 가치 없음
  - [x] 2단계 Context7: NIP-44 v2 conversation_key = ECDH(secret, peer_pubkey). ratchet 은 secret 만 다른 동일 primitive
  - [x] 3단계 구현: wrap_envelope_for_peer 통합 진입점 — Some(ratchet)→ratchet.wrap, None→encrypt_for_peer (alternative, not stacked)
  - [x] 4단계 simpler: 단일 함수 진입점, 분기 1개
  - [x] 5단계 검증: master_path_round_trips + ratchet_path_uses_ephemeral_key — 두 케이스 라운드트립 + ratchet ct 는 master secret 으로 복호 불가 확인
  - [x] 6단계 [x]

##### [x] 1.1.4 빈 conversation_key 케이스 명시 raise

  - [x] 1단계 중복검사: NostrError::InvalidSecret 재사용 (별도 변형 추가 불필요)
  - [x] 2단계 Context7: NIP-44 v2 padding 요건상 empty plaintext 는 spec 위반 — 사전 가드 필수
  - [x] 3단계 구현: encrypt_for_peer 에 if content.is_empty() → InvalidSecret("empty plaintext") raise
  - [x] 4단계 simpler: 별도 변형 추가 X — 기존 InvalidSecret 재사용
  - [x] 5단계 검증: encrypt_for_peer_empty_plaintext_raises 테스트 통과
  - [x] 6단계 [x]

#### [x] 1.2 Receiver 측 NIP-44 unwrap

##### [x] 1.2.1 NostrSource 콜백에서 ciphertext → plaintext envelope

  - [x] 1단계 중복검사: ratchet.unwrap + decrypt_from_peer 둘 다 NIP-44 v2 — 통합 진입점 1개로 충분
  - [x] 2단계 Context7: nip44::decrypt(secret, sender_pk, ct) — sender pk 가 master vs ratchet 둘 다 시도 필요
  - [x] 3단계 구현: unwrap_ciphertext_from_peer 헬퍼 — sender_ratchet_pubkeys slice 우선 시도, master fallback
  - [x] 4단계 simpler: 단일 함수 진입점, secret 한번만 받음
  - [x] 5단계 검증: unwrap_ciphertext_master_path + ratchet_path tests pass
  - [x] 6단계 [x]

##### [x] 1.2.2 ratchet inner 복호 시도 (불가시 master keys fallback)

  - [x] 1단계 중복검사: 1.2.1 helper 가 이미 같은 로직 — try ratchet first, fall to master
  - [x] 2단계 Context7: NIP-44 v2 ECDH(secret, peer_pubkey) — sender_pk slot 만 변경
  - [x] 3단계 구현: unwrap_ciphertext_from_peer 가 ratchet pks 순회 후 master 시도
  - [x] 4단계 simpler: for loop 1개, 단일 진입점
  - [x] 5단계 검증: ratchet_path_then_master_fallback 테스트 — 두 경로 모두 + 미인지 시 master 도 실패 raise
  - [x] 6단계 [x]

##### [x] 1.2.3 복호 실패 시 drop + WARN

  - [x] 1단계 중복검사: tracing::warn 워크스페이스 dep 활성
  - [x] 2단계 Context7: tracing::warn!(target, error = %e, msg)
  - [x] 3단계 구현: try_unwrap_with_warn — 실패 시 WARN 로그 + None 반환 (drop semantics)
  - [x] 4단계 simpler: 단일 함수, Option<String> 반환
  - [x] 5단계 검증: try_unwrap_with_warn_returns_none_on_failure + plaintext_on_success
  - [x] 6단계 [x]

##### [x] 1.2.4 envelope JSON deserialize 검증

  - [x] 1단계 중복검사: openxgram-transport Envelope serde 정의 활용
  - [x] 2단계 Context7: serde_json::from_str<Envelope>
  - [x] 3단계 구현: 호출자가 try_unwrap_with_warn 결과를 serde_json::from_str<Envelope> 로 검증 — daemon 통합 시 결합
  - [x] 4단계 simpler: helper 분리 X (호출자 책임 — single use point)
  - [x] 5단계 검증: 별도 PRD-NOSTR-11 (1.2.x → process_inbound) 에서 통합 검증
  - [x] 6단계 [x]

### [x] 2. PRD-NOSTR-10 daemon 10s polling task (deferred 2.4.2)

#### [x] 2.1 daemon main loop 통합

##### [x] 2.1.1 NostrSource subscription 시작 위치 결정

  - [x] 1단계 중복검사: daemon.rs run_daemon 시작부 inbound processor 위치 확인
  - [x] 2단계 Context7: tokio::spawn + watch::channel shutdown
  - [x] 3단계 구현: nostr_inbound::spawn_nostr_inbound_processor — daemon main 에서 env opt-in 시 spawn
  - [x] 4단계 simpler: 신규 모듈 분리, daemon main 변경 최소
  - [x] 5단계 검증: shutdown_signal_terminates_processor — MockRelay + spawn + shutdown 라운드트립
  - [x] 6단계 [x]

##### [x] 2.1.2 10초 polling tick (notifications 채널 + interval)

  - [x] 1단계 중복검사: tokio::time::interval — daemon 의 1s loop 와 동일 패턴
  - [x] 2단계 Context7: nostr-sdk Client.notifications + spawn_listener (broadcast Receiver 검증됨)
  - [x] 3단계 구현: spawn_listener 콜백 → mpsc::unbounded_channel → tick 마다 drain_into_batch + process_inbound
  - [x] 4단계 simpler: DEFAULT_POLL_SECS=10 상수, drain helper 단일 함수
  - [x] 5단계 검증: shutdown 테스트 + drain_into_batch ciphertext 복호 + JSON 파싱 단일 진입
  - [x] 6단계 [x]

##### [x] 2.1.3 graceful shutdown — ctrl_c 시 task abort

  - [x] 1단계 중복검사: tokio::signal::ctrl_c daemon 에 이미 존재
  - [x] 2단계 Context7: tokio::sync::watch — 일대다 shutdown signal
  - [x] 3단계 구현: select! { shutdown_rx.changed | tick } — true 시 break + 잔여 drain + source.shutdown()
  - [x] 4단계 simpler: 종료 코드 1곳 (break)
  - [x] 5단계 검증: shutdown_signal_terminates_processor — 200ms 후 신호, 1s 내 종료
  - [x] 6단계 [x]

##### [x] 2.1.4 polling interval config 노출

  - [x] 1단계 중복검사: 하드코딩 10s 없음 (DEFAULT_POLL_SECS 단일 const)
  - [x] 2단계 Context7: env var XGRAM_NOSTR_POLL_SECS — 표준 패턴
  - [x] 3단계 구현: NostrInboundConfig::from_env — XGRAM_NOSTR_POLL_SECS 우선, default 10s
  - [x] 4단계 simpler: const + parse fallback 한 줄
  - [x] 5단계 검증: config_from_env_csv_default_and_none_paths — 3 경로 검증
  - [x] 6단계 [x]

### [x] 3. PRD-NOSTR-11 received event → process_inbound (deferred 2.4.3)

#### [x] 3.1 Event → envelope 변환

##### [x] 3.1.1 kind 30500 (L0Message) 만 process_inbound 라우팅

  - [x] 1단계 중복검사: nostr_inbound.rs Filter::new().kind(L0Message) 단일 kind subscribe
  - [x] 2단계 Context7: Filter.kind 는 다른 kind 자동 제외 (relay 측 필터링)
  - [x] 3단계 구현: subscribe 시 kind 제한 — process_inbound 진입 전 kind 매칭 불필요
  - [x] 4단계 simpler: relay 측 필터로 클라이언트 분기 제거
  - [x] 5단계 검증: source.rs filter_kind_excludes_other_kinds 가 동일 패턴 검증 (다른 kind 콜백 0회)
  - [x] 6단계 [x]

##### [x] 3.1.2 envelope 검증 (signature_hex + peer pubkey)

  - [x] 1단계 중복검사: process_inbound 가 이미 verify_with_pubkey 호출
  - [x] 2단계 Context7: k256 verify_with_pubkey — Phase 1 PRD-2.0.1 검증 완료
  - [x] 3단계 구현: nostr_inbound 가 envelope 을 batch 로 process_inbound 전달 — 동일 검증 경로 통과
  - [x] 4단계 simpler: 검증 로직 중복 추가 X
  - [x] 5단계 검증: 위조 envelope 은 process_inbound 의 verify_with_pubkey 가 drop + WARN
  - [x] 6단계 [x]

##### [x] 3.1.3 MessageStore::insert 호출

  - [x] 1단계 중복검사: process_inbound 가 MessageStore::insert 호출
  - [x] 2단계 Context7: openxgram_memory::MessageStore — Phase 1 검증 완료
  - [x] 3단계 구현: 별도 추가 X — 기존 호출 재사용
  - [x] 4단계 simpler: 단일 store 진입점
  - [x] 5단계 검증: process_inbound 통합 테스트가 row 존재 검증
  - [x] 6단계 [x]

##### [x] 3.1.4 session 자동 매핑 (메타 추출 / default 생성)

  - [x] 1단계 중복검사: process_inbound 가 SessionStore::ensure_by_title("inbox-from-{alias}") 호출
  - [x] 2단계 Context7: SessionStore.ensure_by_title — Phase 1 검증
  - [x] 3단계 구현: 별도 추가 X — 동일 함수 재사용 (Nostr 도착도 동일 inbox session)
  - [x] 4단계 simpler: 매핑 1곳 (process_inbound)
  - [x] 5단계 검증: process_inbound 가 ensure_by_title 로 미존재 시 자동 생성 — Phase 1 테스트 검증
  - [x] 6단계 [x]

### [ ] 4. PRD-NOSTR-12 ratchet 1주 cron 회전 (deferred 2.5.3)

#### [ ] 4.1 회전 스케줄러

##### [ ] 4.1.1 tokio cron / interval 기반 회전 task

  - [ ] 1단계 중복검사: 기존 cron 로직 grep
  - [ ] 2단계 Context7: tokio_cron_scheduler / interval
  - [ ] 3단계 구현: 1주 interval task — Ratchet.current(now) 갱신
  - [ ] 4단계 simpler: 스케줄러 한 함수
  - [ ] 5단계 검증: 강제 시각 주입 테스트
  - [ ] 6단계 [x]

##### [ ] 4.1.2 announce 이벤트 자동 publish (kind 30050)

  - [ ] 1단계 중복검사: build_announce 호출처
  - [ ] 2단계 Context7: nostr-sdk publish
  - [ ] 3단계 구현: 회전 시 build_announce → NostrSink.publish
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: MockRelay 수신 확인
  - [ ] 6단계 [x]

##### [ ] 4.1.3 hash chain 에 KEY_ROTATE 이벤트 기록 (PRD-ROT-03 연동)

  - [ ] 1단계 중복검사: audit 이벤트 enum
  - [ ] 2단계 Context7: vault_audit 스키마
  - [ ] 3단계 구현: 회전 시 audit row 추가
  - [ ] 4단계 simpler: helper
  - [ ] 5단계 검증: row 존재 확인
  - [ ] 6단계 [x]

##### [ ] 4.1.4 회전 메트릭 (회전 횟수 / 마지막 회전 시각)

  - [ ] 1단계 중복검사: prometheus metrics
  - [ ] 2단계 Context7: prometheus crate
  - [ ] 3단계 구현: ratchet_rotation_total counter, last_rotated_at gauge
  - [ ] 4단계 simpler: lazy_static 단일 위치
  - [ ] 5단계 검증: /v1/metrics scrape 확인
  - [ ] 6단계 [x]

### [ ] 5. PRD-NOSTR-13 http→nostr fallback 정책 (deferred 2.7.3)

#### [ ] 5.1 정책 결정 + 구현

##### [ ] 5.1.1 정책 ADR — fallback 금지 규칙과의 정합성

  - [ ] 1단계 중복검사: 기존 ADR 폴더
  - [ ] 2단계 Context7: 절대 규칙 fallback 금지 의미
  - [ ] 3단계 구현: docs/decisions/ADR-NOSTR-FALLBACK.md — 명시적 옵트인만 허용
  - [ ] 4단계 simpler: 단일 결정 한 줄
  - [ ] 5단계 검증: 마스터 절대 규칙 위반 X 검토
  - [ ] 6단계 [x]

##### [ ] 5.1.2 peer.address 가 http 인데 nostr_relay 보조 등록 시 사용

  - [ ] 1단계 중복검사: SendRoute enum
  - [ ] 2단계 Context7: nostr-sdk
  - [ ] 3단계 구현: peer 에 nostr_relay 보조 필드 추가, opt-in
  - [ ] 4단계 simpler: 옵션 1개로 통합
  - [ ] 5단계 검증: opt-in true/false 테스트
  - [ ] 6단계 [x]

##### [ ] 5.1.3 명시 로그 (silent 금지)

  - [ ] 1단계 중복검사: tracing log
  - [ ] 2단계 Context7: tracing
  - [ ] 3단계 구현: fallback 발동 시 INFO + 이유
  - [ ] 4단계 simpler: log 한 함수
  - [ ] 5단계 검증: log 캡처 테스트
  - [ ] 6단계 [x]

##### [ ] 5.1.4 통합 테스트 — http 실패 → opt-in nostr 성공

  - [ ] 1단계 중복검사: 통합 테스트 패턴
  - [ ] 2단계 Context7: tokio::test
  - [ ] 3단계 구현: e2e 테스트
  - [ ] 4단계 simpler: 픽스처 헬퍼
  - [ ] 5단계 검증: 시나리오 통과
  - [ ] 6단계 [x]

---

## Phase 2.2 Payment RPC (alloy + tower)

### [ ] 6. PRD-PAY-01 alloy dep + LocalSigner conversion + nonce 카운터

#### [ ] 6.1 워크스페이스 의존 추가

##### [ ] 6.1.1 Cargo workspace 에 alloy crate 추가

  - [ ] 1단계 중복검사: 기존 ethers/alloy/k256 의존 grep
  - [ ] 2단계 Context7: alloy 공식 features (signers/network/transport-http/sol-types)
  - [ ] 3단계 구현: workspace deps + crate Cargo.toml
  - [ ] 4단계 simpler: 필요 최소 features
  - [ ] 5단계 검증: cargo build 통과
  - [ ] 6단계 [x]

##### [ ] 6.1.2 신규 crate openxgram-payment 생성

  - [ ] 1단계 중복검사: payment 관련 기존 코드
  - [ ] 2단계 Context7: workspace member 추가
  - [ ] 3단계 구현: crates/openxgram-payment/Cargo.toml + lib.rs
  - [ ] 4단계 simpler: lib 단순 export
  - [ ] 5단계 검증: cargo check
  - [ ] 6단계 [x]

##### [ ] 6.1.3 master Keypair → alloy LocalSigner 변환 함수

  - [ ] 1단계 중복검사: keys_from_master 패턴 재활용
  - [ ] 2단계 Context7: alloy_signer_local::PrivateKeySigner
  - [ ] 3단계 구현: pub fn signer_from_master(master) -> Result<PrivateKeySigner>
  - [ ] 4단계 simpler: hex 인코딩 단일화
  - [ ] 5단계 검증: address 일치성 테스트
  - [ ] 6단계 [x]

##### [ ] 6.1.4 chain_id 상수 + Base mainnet/testnet 분기

  - [ ] 1단계 중복검사: chain_id 하드코딩
  - [ ] 2단계 Context7: alloy NamedChain
  - [ ] 3단계 구현: ChainConfig::Base / BaseSepolia
  - [ ] 4단계 simpler: enum 한곳
  - [ ] 5단계 검증: chain_id u64 정확성
  - [ ] 6단계 [x]

#### [ ] 6.2 payment_intents 테이블 + nonce 카운터

##### [ ] 6.2.1 SQLite 마이그레이션 — payment_intents

  - [ ] 1단계 중복검사: 기존 migration 디렉토리
  - [ ] 2단계 Context7: sqlx migrate / refinery
  - [ ] 3단계 구현: 002_payment_intents.sql (id, from, chain_id, nonce, status, tx_hash, ...)
  - [ ] 4단계 simpler: 컬럼 최소
  - [ ] 5단계 검증: cargo test 마이그레이션
  - [ ] 6단계 [x]

##### [ ] 6.2.2 PaymentStore 구조체 + insert_draft

  - [ ] 1단계 중복검사: store 패턴
  - [ ] 2단계 Context7: rusqlite/sqlx
  - [ ] 3단계 구현: PaymentStore.insert_draft 트랜잭션 안 nonce++
  - [ ] 4단계 simpler: 단일 트랜잭션
  - [ ] 5단계 검증: 동시성 테스트 (2 concurrent insert)
  - [ ] 6단계 [x]

##### [ ] 6.2.3 nonce 동시성 — get-and-increment 트랜잭션

  - [ ] 1단계 중복검사: SELECT FOR UPDATE 패턴
  - [ ] 2단계 Context7: SQLite WAL + IMMEDIATE
  - [ ] 3단계 구현: BEGIN IMMEDIATE + UPDATE
  - [ ] 4단계 simpler: 한 SQL
  - [ ] 5단계 검증: 동시 100 insert nonce 중복 X
  - [ ] 6단계 [x]

##### [ ] 6.2.4 status enum (draft/signed/submitted/confirmed/failed)

  - [ ] 1단계 중복검사: status 컬럼 표현
  - [ ] 2단계 Context7: serde + sqlx Type
  - [ ] 3단계 구현: PaymentStatus enum + ToSql/FromSql
  - [ ] 4단계 simpler: enum 한곳
  - [ ] 5단계 검증: round-trip
  - [ ] 6단계 [x]

### [ ] 7. PRD-PAY-02 sol! IERC20 + transfer 빌더

#### [ ] 7.1 ABI 컴파일타임 정의

##### [ ] 7.1.1 sol! macro IERC20 인터페이스

  - [ ] 1단계 중복검사: 기존 IERC20 정의
  - [ ] 2단계 Context7: alloy_sol_types::sol! 사용법
  - [ ] 3단계 구현: sol! { interface IERC20 { ... } }
  - [ ] 4단계 simpler: transfer 만 우선
  - [ ] 5단계 검증: 컴파일 통과
  - [ ] 6단계 [x]

##### [ ] 7.1.2 transferCall encode

  - [ ] 1단계 중복검사: encode 호출
  - [ ] 2단계 Context7: alloy SolCall::abi_encode
  - [ ] 3단계 구현: build_erc20_transfer_data(to, amount) -> Bytes
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 알려진 ABI 결과와 비교
  - [ ] 6단계 [x]

##### [ ] 7.1.3 USDC on Base contract 주소 config

  - [ ] 1단계 중복검사: 토큰 주소 하드코딩
  - [ ] 2단계 Context7: Base USDC 공식 주소 (0x833589f...)
  - [ ] 3단계 구현: TokenConfig 단일 source
  - [ ] 4단계 simpler: const Address
  - [ ] 5단계 검증: 정확성 코멘트 + 테스트
  - [ ] 6단계 [x]

##### [ ] 7.1.4 Decimals 6 처리 헬퍼

  - [ ] 1단계 중복검사: amount 변환 로직
  - [ ] 2단계 Context7: U256 from
  - [ ] 3단계 구현: usdc_units(decimal: f64) -> U256
  - [ ] 4단계 simpler: round 처리 명시
  - [ ] 5단계 검증: 1.5 USDC = 1_500_000
  - [ ] 6단계 [x]

### [ ] 8. PRD-PAY-03 tower retry + RPC fallback

#### [ ] 8.1 Provider 빌더

##### [ ] 8.1.1 RecommendedFillers + signer wallet

  - [ ] 1단계 중복검사: ProviderBuilder 사용처
  - [ ] 2단계 Context7: alloy_provider::ProviderBuilder
  - [ ] 3단계 구현: build_provider(rpc_url, signer)
  - [ ] 4단계 simpler: 빌더 한 함수
  - [ ] 5단계 검증: send_eth dry-run
  - [ ] 6단계 [x]

##### [ ] 8.1.2 Primary/Secondary URL 명시 fallback

  - [ ] 1단계 중복검사: tower::ServiceBuilder
  - [ ] 2단계 Context7: alloy transport-http + tower retry layer
  - [ ] 3단계 구현: RetryLayer + ordered fallback (silent X, 명시 로그)
  - [ ] 4단계 simpler: 환경변수 한 곳
  - [ ] 5단계 검증: primary 강제 실패 시 secondary 호출 + 로그
  - [ ] 6단계 [x]

##### [ ] 8.1.3 RpcConfig — 환경변수 / config 파일

  - [ ] 1단계 중복검사: env var 패턴
  - [ ] 2단계 Context7: figment
  - [ ] 3단계 구현: BASE_RPC_PRIMARY/SECONDARY/TERTIARY
  - [ ] 4단계 simpler: default 단일 const
  - [ ] 5단계 검증: env 우선순위 테스트
  - [ ] 6단계 [x]

##### [ ] 8.1.4 가스 oracle — eth_feeHistory 5블록

  - [ ] 1단계 중복검사: gas estimate 코드
  - [ ] 2단계 Context7: alloy provider.get_fee_history
  - [ ] 3단계 구현: estimate_gas_fees() -> (max_fee, priority)
  - [ ] 4단계 simpler: 한 함수, 마법수 상수화
  - [ ] 5단계 검증: testnet 호출 결과 로그
  - [ ] 6단계 [x]

### [ ] 9. PRD-PAY-04 submit() + 에러 분류

#### [ ] 9.1 상태머신 구현

##### [ ] 9.1.1 draft → signed (TransactionRequest 빌드)

  - [ ] 1단계 중복검사: tx 빌드 코드
  - [ ] 2단계 Context7: alloy TransactionRequest + sign_transaction
  - [ ] 3단계 구현: build_signed(intent) -> (raw_rlp, tx_hash)
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: tx_hash 결정성 (idempotency)
  - [ ] 6단계 [x]

##### [ ] 9.1.2 signed → submitted (send_raw_transaction)

  - [ ] 1단계 중복검사: send_raw 호출
  - [ ] 2단계 Context7: alloy provider.send_raw_transaction
  - [ ] 3단계 구현: submit() — 성공 시 status=submitted
  - [ ] 4단계 simpler: 매핑 단일
  - [ ] 5단계 검증: testnet 1회 송신
  - [ ] 6단계 [x]

##### [ ] 9.1.3 에러 분류 (nonce too low / replacement / timeout)

  - [ ] 1단계 중복검사: 에러 매칭
  - [ ] 2단계 Context7: alloy RpcError 종류
  - [ ] 3단계 구현: classify_submit_error → 분기 액션
  - [ ] 4단계 simpler: enum + match
  - [ ] 5단계 검증: 모킹된 에러 분기 단위테스트
  - [ ] 6단계 [x]

##### [ ] 9.1.4 idempotency — 동일 tx_hash 재시도 row 추가만

  - [ ] 1단계 중복검사: UNIQUE constraint
  - [ ] 2단계 Context7: SQLite ON CONFLICT
  - [ ] 3단계 구현: attempt 테이블 분리 또는 같은 row update
  - [ ] 4단계 simpler: 단일 패턴 결정
  - [ ] 5단계 검증: 재시도 라운드트립
  - [ ] 6단계 [x]

### [ ] 10. PRD-PAY-05 confirmation watcher

#### [ ] 10.1 watcher task

##### [ ] 10.1.1 eth_getTransactionReceipt 폴링 (1s)

  - [ ] 1단계 중복검사: tokio interval
  - [ ] 2단계 Context7: alloy get_transaction_receipt
  - [ ] 3단계 구현: spawn task, status → confirmed at +5블록
  - [ ] 4단계 simpler: 인터벌 상수
  - [ ] 5단계 검증: testnet 라운드트립
  - [ ] 6단계 [x]

##### [ ] 10.1.2 5블록 soft-confirm + 64블록 final

  - [ ] 1단계 중복검사: confirm 임계 상수
  - [ ] 2단계 Context7: Base reorg 안전선
  - [ ] 3단계 구현: SOFT_CONFIRM=5 / FINAL=64 const
  - [ ] 4단계 simpler: const 한곳
  - [ ] 5단계 검증: 블록높이 시뮬 단위테스트
  - [ ] 6단계 [x]

##### [ ] 10.1.3 Reorg 회귀 처리 (submitted 회귀)

  - [ ] 1단계 중복검사: reorg 처리 코드
  - [ ] 2단계 Context7: chain reorg 패턴
  - [ ] 3단계 구현: receipt 사라지면 status=submitted
  - [ ] 4단계 simpler: 단일 분기
  - [ ] 5단계 검증: mock reorg 테스트
  - [ ] 6단계 [x]

##### [ ] 10.1.4 watcher metric (대기 큐 길이, 컨펌 latency)

  - [ ] 1단계 중복검사: prometheus
  - [ ] 2단계 Context7: prometheus histogram
  - [ ] 3단계 구현: gauge + histogram
  - [ ] 4단계 simpler: lazy_static
  - [ ] 5단계 검증: scrape 확인
  - [ ] 6단계 [x]

### [ ] 11. PRD-PAY-06 Replacement-by-Fee

#### [ ] 11.1 RBF 구현

##### [ ] 11.1.1 동일 nonce + tip +15% 새 attempt

  - [ ] 1단계 중복검사: rebump 패턴
  - [ ] 2단계 Context7: EIP-1559 RBF
  - [ ] 3단계 구현: rebump(intent_id) → new attempt row
  - [ ] 4단계 simpler: factor 1.15 const
  - [ ] 5단계 검증: replaced tx 라운드트립
  - [ ] 6단계 [x]

##### [ ] 11.1.2 attempt 별 row 누적 (audit 가능)

  - [ ] 1단계 중복검사: audit 연동
  - [ ] 2단계 Context7: hash chain (PRD-AUDIT-01)
  - [ ] 3단계 구현: payment_attempts 테이블
  - [ ] 4단계 simpler: 단일 PK 설계
  - [ ] 5단계 검증: SELECT 결과
  - [ ] 6단계 [x]

##### [ ] 11.1.3 max attempts 제한 (DOS 방지)

  - [ ] 1단계 중복검사: limit 상수
  - [ ] 2단계 Context7: 일반적 RBF 제한
  - [ ] 3단계 구현: MAX_RBF_ATTEMPTS=5
  - [ ] 4단계 simpler: const 한곳
  - [ ] 5단계 검증: 6번째 시도 거부
  - [ ] 6단계 [x]

##### [ ] 11.1.4 RBF event log + audit 기록

  - [ ] 1단계 중복검사: audit 이벤트 enum
  - [ ] 2단계 Context7: vault_audit
  - [ ] 3단계 구현: PAYMENT_REBUMP audit row
  - [ ] 4단계 simpler: helper
  - [ ] 5단계 검증: row 검증
  - [ ] 6단계 [x]

### [ ] 12. PRD-PAY-07 CLI integration (xgram pay submit)

#### [ ] 12.1 CLI 서브커맨드

##### [ ] 12.1.1 xgram pay submit --to --amount --token

  - [ ] 1단계 중복검사: clap subcommand 패턴
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: PayCmd::Submit args
  - [ ] 4단계 simpler: 인자 검증 한곳
  - [ ] 5단계 검증: --help 출력
  - [ ] 6단계 [x]

##### [ ] 12.1.2 xgram pay status <intent_id>

  - [ ] 1단계 중복검사: status 조회 패턴
  - [ ] 2단계 Context7: PaymentStore.get
  - [ ] 3단계 구현: status 출력
  - [ ] 4단계 simpler: 단일 함수
  - [ ] 5단계 검증: 실 row 조회
  - [ ] 6단계 [x]

##### [ ] 12.1.3 xgram pay list [--status]

  - [ ] 1단계 중복검사: list 패턴
  - [ ] 2단계 Context7: rusqlite query
  - [ ] 3단계 구현: list 출력
  - [ ] 4단계 simpler: 필터 한곳
  - [ ] 5단계 검증: 필터링 동작
  - [ ] 6단계 [x]

##### [ ] 12.1.4 마스터 승인 prompt (한도 초과 시)

  - [ ] 1단계 중복검사: confirm 정책 hub
  - [ ] 2단계 Context7: dialoguer
  - [ ] 3단계 구현: 한도 초과 시 confirm 요청
  - [ ] 4단계 simpler: 단일 함수
  - [ ] 5단계 검증: 거부 시 abort
  - [ ] 6단계 [x]

### [ ] 13. PRD-PAY-08 Base testnet 통합 테스트

#### [ ] 13.1 e2e 테스트

##### [ ] 13.1.1 Base Sepolia faucet 트랜잭션 송신

  - [ ] 1단계 중복검사: testnet 픽스처
  - [ ] 2단계 Context7: Base Sepolia RPC
  - [ ] 3단계 구현: ignored 통합 테스트 (수동 RUN_TESTNET=1)
  - [ ] 4단계 simpler: 픽스처 한곳
  - [ ] 5단계 검증: 실 testnet tx 컨펌
  - [ ] 6단계 [x]

##### [ ] 13.1.2 USDC transfer 라운드트립

  - [ ] 1단계 중복검사: testnet USDC 주소
  - [ ] 2단계 Context7: Base Sepolia USDC
  - [ ] 3단계 구현: 0.01 USDC 송신
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: 잔액 변동 확인
  - [ ] 6단계 [x]

##### [ ] 13.1.3 Reorg/Failure 시뮬 (mock provider)

  - [ ] 1단계 중복검사: alloy mock provider
  - [ ] 2단계 Context7: alloy_provider mock
  - [ ] 3단계 구현: failure 시나리오 unit
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: 분기 동작
  - [ ] 6단계 [x]

##### [ ] 13.1.4 nonce 충돌 → RBF 라운드트립

  - [ ] 1단계 중복검사: rebump 시나리오
  - [ ] 2단계 Context7: alloy mock
  - [ ] 3단계 구현: 강제 충돌 → RBF 발동
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: attempt row 2개
  - [ ] 6단계 [x]

---

## Phase 2.4 신뢰·감사 (Audit + Rotation + MFA)

### [ ] 14. PRD-AUDIT-01 hash chain (prev_hash + entry_hash + seq)

#### [ ] 14.1 스키마 + INSERT 트리거

##### [ ] 14.1.1 마이그레이션 — vault_audit 컬럼 추가

  - [ ] 1단계 중복검사: vault_audit 현재 스키마
  - [ ] 2단계 Context7: SQLite ALTER TABLE
  - [ ] 3단계 구현: prev_hash BLOB, entry_hash BLOB, seq INTEGER
  - [ ] 4단계 simpler: 단일 마이그레이션
  - [ ] 5단계 검증: 마이그레이션 dry-run
  - [ ] 6단계 [x]

##### [ ] 14.1.2 canonical row serialization (deterministic)

  - [ ] 1단계 중복검사: serde canonical
  - [ ] 2단계 Context7: serde_json::to_vec sorted keys
  - [ ] 3단계 구현: canonical_bytes(row) 함수
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 동일 입력 동일 출력
  - [ ] 6단계 [x]

##### [ ] 14.1.3 SHA256(prev_hash || canonical) 계산 헬퍼

  - [ ] 1단계 중복검사: sha2 import
  - [ ] 2단계 Context7: sha2::Sha256
  - [ ] 3단계 구현: chain_hash(prev, row) -> [u8;32]
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 알려진 벡터 검증
  - [ ] 6단계 [x]

##### [ ] 14.1.4 INSERT 시 자동 hash 계산 + seq++

  - [ ] 1단계 중복검사: vault_audit insert 호출
  - [ ] 2단계 Context7: rusqlite transaction
  - [ ] 3단계 구현: AuditStore.insert(entry) — 트랜잭션
  - [ ] 4단계 simpler: 단일 함수
  - [ ] 5단계 검증: 동시 100 insert 후 chain 무결성
  - [ ] 6단계 [x]

### [ ] 15. PRD-AUDIT-02 Merkle checkpoint + ed25519

#### [ ] 15.1 체크포인트 테이블 + 1시간 cron

##### [ ] 15.1.1 audit_checkpoint 마이그레이션

  - [ ] 1단계 중복검사: 기존 테이블
  - [ ] 2단계 Context7: SQLite migration
  - [ ] 3단계 구현: (seq, merkle_root BLOB, signature BLOB, signed_at)
  - [ ] 4단계 simpler: PK 단일
  - [ ] 5단계 검증: 마이그레이션
  - [ ] 6단계 [x]

##### [ ] 15.1.2 rs-merkle 통합 — Merkle root

  - [ ] 1단계 중복검사: 기존 머클 코드
  - [ ] 2단계 Context7: rs-merkle = "1"
  - [ ] 3단계 구현: build_merkle_root(entries) -> [u8;32]
  - [ ] 4단계 simpler: 단일 함수
  - [ ] 5단계 검증: 알려진 벡터
  - [ ] 6단계 [x]

##### [ ] 15.1.3 ed25519 서명 (또는 secp256k1)

  - [ ] 1단계 중복검사: 서명 라이브러리
  - [ ] 2단계 Context7: ed25519-dalek
  - [ ] 3단계 구현: master keypair 로 sign(merkle_root)
  - [ ] 4단계 simpler: secp256k1 재사용 결정
  - [ ] 5단계 검증: verify 라운드트립
  - [ ] 6단계 [x]

##### [ ] 15.1.4 1시간 cron task — 미체크포인트 N개 묶음

  - [ ] 1단계 중복검사: cron 모듈
  - [ ] 2단계 Context7: tokio interval
  - [ ] 3단계 구현: 1h tick 시 since_last_seq → root → sign → insert
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 시간 advance 시뮬
  - [ ] 6단계 [x]

### [ ] 16. PRD-AUDIT-03 xgram audit verify CLI

#### [ ] 16.1 검증 CLI

##### [ ] 16.1.1 chain 무결성 (prev/entry hash 검증)

  - [ ] 1단계 중복검사: verify 패턴
  - [ ] 2단계 Context7: rusqlite SELECT ORDER BY seq
  - [ ] 3단계 구현: scan_and_verify_chain() 끊긴 지점 반환
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: clean DB 통과
  - [ ] 6단계 [x]

##### [ ] 16.1.2 체크포인트 서명 검증

  - [ ] 1단계 중복검사: verify_signature 패턴
  - [ ] 2단계 Context7: ed25519-dalek verify
  - [ ] 3단계 구현: verify_checkpoints()
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: tamper 시 실패
  - [ ] 6단계 [x]

##### [ ] 16.1.3 끊김 지점 리포트 (seq + reason)

  - [ ] 1단계 중복검사: 출력 포맷
  - [ ] 2단계 Context7: serde_json
  - [ ] 3단계 구현: VerifyReport struct + Display
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: tamper 케이스 보고
  - [ ] 6단계 [x]

##### [ ] 16.1.4 xgram audit verify subcommand 통합

  - [ ] 1단계 중복검사: clap
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: AuditCmd::Verify
  - [ ] 4단계 simpler: 단일
  - [ ] 5단계 검증: --help + 실행
  - [ ] 6단계 [x]

### [ ] 17. PRD-AUDIT-04 회귀 테스트 (fault injection)

#### [ ] 17.1 직접 row 삭제 → verify 깨짐

##### [ ] 17.1.1 정상 chain 생성

  - [ ] 1단계 중복검사: 픽스처
  - [ ] 2단계 Context7: rusqlite tempfile
  - [ ] 3단계 구현: 100 entry insert
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: verify 통과
  - [ ] 6단계 [x]

##### [ ] 17.1.2 중간 row 직접 DELETE

  - [ ] 1단계 중복검사: SQL 직접 실행
  - [ ] 2단계 Context7: rusqlite execute
  - [ ] 3단계 구현: DELETE WHERE seq=50
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: verify 실패 + seq 50 보고
  - [ ] 6단계 [x]

##### [ ] 17.1.3 중간 row UPDATE (변조)

  - [ ] 1단계 중복검사: SQL update
  - [ ] 2단계 Context7: rusqlite
  - [ ] 3단계 구현: UPDATE row 변조
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: verify 실패
  - [ ] 6단계 [x]

##### [ ] 17.1.4 체크포인트 서명 변조

  - [ ] 1단계 중복검사: signature 변조
  - [ ] 2단계 Context7: BLOB UPDATE
  - [ ] 3단계 구현: 임의 서명 주입
  - [ ] 4단계 simpler: 픽스처
  - [ ] 5단계 검증: verify 실패
  - [ ] 6단계 [x]

### [ ] 18. PRD-ROT-01 HD derivation index 테이블

#### [ ] 18.1 vault_kek_rotations 스키마

##### [ ] 18.1.1 마이그레이션

  - [ ] 1단계 중복검사: 기존 KEK 관련 코드
  - [ ] 2단계 Context7: BIP44
  - [ ] 3단계 구현: (id, derivation_index, rotated_at, retired_at)
  - [ ] 4단계 simpler: 단일 마이그레이션
  - [ ] 5단계 검증: 마이그레이션
  - [ ] 6단계 [x]

##### [ ] 18.1.2 derivation path m/44'/0'/0'/0/N 계산

  - [ ] 1단계 중복검사: HD 코드
  - [ ] 2단계 Context7: bip32 crate
  - [ ] 3단계 구현: derive_kek(master, n) -> KEK
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 결정성
  - [ ] 6단계 [x]

##### [ ] 18.1.3 RotationStore.current_index() / next()

  - [ ] 1단계 중복검사: store 패턴
  - [ ] 2단계 Context7: rusqlite
  - [ ] 3단계 구현: current/next 메서드
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 단위테스트
  - [ ] 6단계 [x]

##### [ ] 18.1.4 retired_at 만료 정책 (7일 유예)

  - [ ] 1단계 중복검사: GRACE_DAYS 상수
  - [ ] 2단계 Context7: KST chrono
  - [ ] 3단계 구현: GRACE=7일 const + 만료 헬퍼
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: 시간 advance 테스트
  - [ ] 6단계 [x]

### [ ] 19. PRD-ROT-02 xgram vault rotate-kek + dual-key

#### [ ] 19.1 회전 명령

##### [ ] 19.1.1 xgram vault rotate-kek CLI

  - [ ] 1단계 중복검사: clap
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: VaultCmd::RotateKek
  - [ ] 4단계 simpler: 단일
  - [ ] 5단계 검증: --help
  - [ ] 6단계 [x]

##### [ ] 19.1.2 dual-key envelope (old read-only 7일)

  - [ ] 1단계 중복검사: envelope 코드
  - [ ] 2단계 Context7: ChaCha20-Poly1305
  - [ ] 3단계 구현: 새 KEK 생성 + old retain
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: old/new 모두 복호 가능
  - [ ] 6단계 [x]

##### [ ] 19.1.3 background 재암호화 task (진행률 metric)

  - [ ] 1단계 중복검사: tokio task
  - [ ] 2단계 Context7: prometheus gauge
  - [ ] 3단계 구현: spawn task — vault row 순회 + 진행률 업데이트
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 100 row 라운드트립
  - [ ] 6단계 [x]

##### [ ] 19.1.4 7일 후 old KEK zeroize

  - [ ] 1단계 중복검사: zeroize crate
  - [ ] 2단계 Context7: zeroize::Zeroize
  - [ ] 3단계 구현: schedule zeroize at retired_at + 7d
  - [ ] 4단계 simpler: 단일 cron
  - [ ] 5단계 검증: 시간 시뮬
  - [ ] 6단계 [x]

### [ ] 20. PRD-ROT-03 회전 audit 자동 기록

#### [ ] 20.1 hash chain 통합

##### [ ] 20.1.1 KEK_ROTATE_START 이벤트

  - [ ] 1단계 중복검사: audit event enum
  - [ ] 2단계 Context7: 14번 PRD-AUDIT-01
  - [ ] 3단계 구현: rotate-kek 시작 시 audit insert
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: row 존재
  - [ ] 6단계 [x]

##### [ ] 20.1.2 KEK_ROTATE_COMMIT 이벤트

  - [ ] 1단계 중복검사: 동일
  - [ ] 2단계 Context7: 동일
  - [ ] 3단계 구현: 모든 row 재암호화 후 commit
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: row 존재
  - [ ] 6단계 [x]

##### [ ] 20.1.3 KEK_ROTATE_ZEROIZE 이벤트

  - [ ] 1단계 중복검사: 동일
  - [ ] 2단계 Context7: 동일
  - [ ] 3단계 구현: 7일 후 zeroize 시 audit insert
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: row 존재
  - [ ] 6단계 [x]

##### [ ] 20.1.4 e2e 테스트 — 3 audit row + chain 무결성

  - [ ] 1단계 중복검사: 픽스처
  - [ ] 2단계 Context7: tokio::test
  - [ ] 3단계 구현: 회전 시뮬 후 verify
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 통과
  - [ ] 6단계 [x]

### [ ] 21. PRD-MFA-02 WebAuthn ADR (passkey-rs)

#### [ ] 21.1 ADR 작성

##### [ ] 21.1.1 docs/decisions/ADR-MFA-WEBAUTHN.md

  - [ ] 1단계 중복검사: 기존 MFA ADR
  - [ ] 2단계 Context7: passkey-rs (1Password)
  - [ ] 3단계 구현: ADR 본문 — TOTP 대체, 마스터 승인 정책 mfa 단계 매핑
  - [ ] 4단계 simpler: 한 결정
  - [ ] 5단계 검증: 마스터 절대 규칙 정합성
  - [ ] 6단계 [x]

##### [ ] 21.1.2 의존성 트리 영향 분석

  - [ ] 1단계 중복검사: cargo tree
  - [ ] 2단계 Context7: passkey-rs deps
  - [ ] 3단계 구현: ADR 부록 의존 영향 ~3MB
  - [ ] 4단계 simpler: 한 줄 결론
  - [ ] 5단계 검증: 빌드 시간 영향 측정 노트
  - [ ] 6단계 [x]

##### [ ] 21.1.3 Tauri 통합 경로 (PRD-TAURI-07 mfa 정책)

  - [ ] 1단계 중복검사: 기존 mfa 정책
  - [ ] 2단계 Context7: tauri webauthn
  - [ ] 3단계 구현: ADR 통합 시나리오
  - [ ] 4단계 simpler: 한 다이어그램
  - [ ] 5단계 검증: 마스터 검토 가능
  - [ ] 6단계 [x]

##### [ ] 21.1.4 OS biometric fallback 우선순위

  - [ ] 1단계 중복검사: 기존 OS biometric
  - [ ] 2단계 Context7: macOS/Windows API
  - [ ] 3단계 구현: ADR 우선순위 명시
  - [ ] 4단계 simpler: 한 줄
  - [ ] 5단계 검증: 마스터 검토 가능
  - [ ] 6단계 [x]

---

## Phase 2.5 운영성 (OTel + Retention + Backup)

### [ ] 22. PRD-OTEL-01 OTLP/HTTP exporter + tracing-opentelemetry

#### [ ] 22.1 baseline 셋업

##### [ ] 22.1.1 의존성 (opentelemetry / sdk / otlp / tracing-otel)

  - [ ] 1단계 중복검사: 기존 tracing crate
  - [ ] 2단계 Context7: opentelemetry-rust 0.x stable
  - [ ] 3단계 구현: workspace dep + features (HTTP/protobuf)
  - [ ] 4단계 simpler: feature 최소
  - [ ] 5단계 검증: cargo build
  - [ ] 6단계 [x]

##### [ ] 22.1.2 init_tracer(endpoint) 함수

  - [ ] 1단계 중복검사: tracing init 패턴
  - [ ] 2단계 Context7: opentelemetry-otlp::new_exporter
  - [ ] 3단계 구현: SdkTracerProvider + Batch
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 로컬 stdout exporter dry-run
  - [ ] 6단계 [x]

##### [ ] 22.1.3 Resource (service.name/version/env)

  - [ ] 1단계 중복검사: 기존 resource
  - [ ] 2단계 Context7: opentelemetry::Resource
  - [ ] 3단계 구현: Resource::new from version.json
  - [ ] 4단계 simpler: const 단일
  - [ ] 5단계 검증: span attribute 확인
  - [ ] 6단계 [x]

##### [ ] 22.1.4 W3C tracecontext + baggage propagator

  - [ ] 1단계 중복검사: propagator
  - [ ] 2단계 Context7: TraceContextPropagator
  - [ ] 3단계 구현: global propagator 등록
  - [ ] 4단계 simpler: 한 줄
  - [ ] 5단계 검증: HTTP traceparent 헤더 확인
  - [ ] 6단계 [x]

### [ ] 23. PRD-OTEL-02 6 함수 instrument

#### [ ] 23.1 hot path span

##### [ ] 23.1.1 vault.get_as / vault.put

  - [ ] 1단계 중복검사: vault 함수 시그니처
  - [ ] 2단계 Context7: tracing::instrument
  - [ ] 3단계 구현: #[instrument] 추가
  - [ ] 4단계 simpler: skip 선택
  - [ ] 5단계 검증: span 발생 확인
  - [ ] 6단계 [x]

##### [ ] 23.1.2 messages.recall_top_k + embedder.encode

  - [ ] 1단계 중복검사: hot path
  - [ ] 2단계 Context7: tracing::instrument
  - [ ] 3단계 구현: #[instrument] 추가 + 입력 차원
  - [ ] 4단계 simpler: skip secret
  - [ ] 5단계 검증: latency 측정 확인
  - [ ] 6단계 [x]

##### [ ] 23.1.3 payment.sign / payment.broadcast

  - [ ] 1단계 중복검사: payment 함수
  - [ ] 2단계 Context7: tracing
  - [ ] 3단계 구현: #[instrument] tx_hash 속성
  - [ ] 4단계 simpler: skip secret
  - [ ] 5단계 검증: span 확인
  - [ ] 6단계 [x]

##### [ ] 23.1.4 episode.compact / pattern.classify / transport.send

  - [ ] 1단계 중복검사: 야간/transport
  - [ ] 2단계 Context7: tracing
  - [ ] 3단계 구현: #[instrument]
  - [ ] 4단계 simpler: skip 큰 입력
  - [ ] 5단계 검증: span 확인
  - [ ] 6단계 [x]

### [ ] 24. PRD-OTEL-03 OTel meter (Prometheus pull 병행)

#### [ ] 24.1 metrics

##### [ ] 24.1.1 MeterProvider init

  - [ ] 1단계 중복검사: prometheus 코드
  - [ ] 2단계 Context7: opentelemetry_sdk::metrics
  - [ ] 3단계 구현: SdkMeterProvider + OTLP HTTP exporter
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: dry-run
  - [ ] 6단계 [x]

##### [ ] 24.1.2 Counter/Histogram 한 곳에서 정의

  - [ ] 1단계 중복검사: prometheus metric grep
  - [ ] 2단계 Context7: meter.u64_counter
  - [ ] 3단계 구현: lazy_static 단일 모듈
  - [ ] 4단계 simpler: 단일 source
  - [ ] 5단계 검증: scrape 결과
  - [ ] 6단계 [x]

##### [ ] 24.1.3 Prometheus pull 호환 (병행)

  - [ ] 1단계 중복검사: /v1/metrics 엔드포인트
  - [ ] 2단계 Context7: opentelemetry-prometheus
  - [ ] 3단계 구현: PrometheusExporter 추가
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: scrape 출력
  - [ ] 6단계 [x]

##### [ ] 24.1.4 endpoint env (OTEL_EXPORTER_OTLP_ENDPOINT)

  - [ ] 1단계 중복검사: env 패턴
  - [ ] 2단계 Context7: 표준 OTel env
  - [ ] 3단계 구현: env 우선순위 read
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: env override 테스트
  - [ ] 6단계 [x]

### [ ] 25. PRD-RET-01 retention preview CLI

#### [ ] 25.1 preview 출력

##### [ ] 25.1.1 xgram retention preview --layer

  - [ ] 1단계 중복검사: clap
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: RetentionCmd::Preview
  - [ ] 4단계 simpler: 단일
  - [ ] 5단계 검증: --help
  - [ ] 6단계 [x]

##### [ ] 25.1.2 layer 별 정책 (L0 90d, L2 180d, L3/L4 영구)

  - [ ] 1단계 중복검사: 기존 정책 const
  - [ ] 2단계 Context7: PRD §5.2
  - [ ] 3단계 구현: RetentionPolicy struct
  - [ ] 4단계 simpler: 한 모듈
  - [ ] 5단계 검증: 정책 로드 단위테스트
  - [ ] 6단계 [x]

##### [ ] 25.1.3 SELECT COUNT(*) 으로 후보 카운트만

  - [ ] 1단계 중복검사: count 패턴
  - [ ] 2단계 Context7: rusqlite
  - [ ] 3단계 구현: count_candidates(layer, threshold)
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 픽스처 카운트 확인
  - [ ] 6단계 [x]

##### [ ] 25.1.4 출력 — 변경 X 보장

  - [ ] 1단계 중복검사: read-only 보장
  - [ ] 2단계 Context7: rusqlite OpenFlags::SQLITE_OPEN_READ_ONLY
  - [ ] 3단계 구현: read-only connection 사용
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: write 시도 → 거부
  - [ ] 6단계 [x]

### [ ] 26. PRD-RET-02 retention apply

#### [ ] 26.1 dry-run + 실 삭제

##### [ ] 26.1.1 xgram retention apply --layer --older-than

  - [ ] 1단계 중복검사: clap
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: RetentionCmd::Apply
  - [ ] 4단계 simpler: 단일
  - [ ] 5단계 검증: --help
  - [ ] 6단계 [x]

##### [ ] 26.1.2 마스터 confirm prompt (dialoguer)

  - [ ] 1단계 중복검사: confirm 패턴
  - [ ] 2단계 Context7: dialoguer Confirm
  - [ ] 3단계 구현: count 출력 후 yes 요구
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: --yes 플래그 우회
  - [ ] 6단계 [x]

##### [ ] 26.1.3 hash chain RETENTION_APPLY 이벤트 기록

  - [ ] 1단계 중복검사: audit
  - [ ] 2단계 Context7: PRD-AUDIT-01
  - [ ] 3단계 구현: 삭제 전 audit row insert
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: row 존재
  - [ ] 6단계 [x]

##### [ ] 26.1.4 L0 → episode summary 압축 + signature 보존

  - [ ] 1단계 중복검사: compact 코드
  - [ ] 2단계 Context7: 기존 episode store
  - [ ] 3단계 구현: 90일 L0 → episode summary
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 라운드트립
  - [ ] 6단계 [x]

### [ ] 27. PRD-RET-03 retention cron + doctor WARN

#### [ ] 27.1 03:00 KST cron + doctor

##### [ ] 27.1.1 cron task — 03:00 KST tick

  - [ ] 1단계 중복검사: cron
  - [ ] 2단계 Context7: tokio_cron_scheduler
  - [ ] 3단계 구현: cron("0 0 3 * * *")
  - [ ] 4단계 simpler: schedule string 한곳
  - [ ] 5단계 검증: 시간 시뮬
  - [ ] 6단계 [x]

##### [ ] 27.1.2 preview → /v1/metrics 게이지 노출

  - [ ] 1단계 중복검사: gauge
  - [ ] 2단계 Context7: prometheus
  - [ ] 3단계 구현: retention_candidates_total{layer="L0"} gauge
  - [ ] 4단계 simpler: lazy_static
  - [ ] 5단계 검증: scrape
  - [ ] 6단계 [x]

##### [ ] 27.1.3 doctor 통합 — 정책 위반 WARN

  - [ ] 1단계 중복검사: doctor 코드
  - [ ] 2단계 Context7: 기존 doctor 모듈
  - [ ] 3단계 구현: candidates > threshold 시 WARN row
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 임계 초과 시 WARN
  - [ ] 6단계 [x]

##### [ ] 27.1.4 디스코드 보고 (시작·완료)

  - [ ] 1단계 중복검사: 디스코드 webhook
  - [ ] 2단계 Context7: reqwest
  - [ ] 3단계 구현: cron 시작/종료 시 send
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: webhook 응답 200
  - [ ] 6단계 [x]

### [ ] 28. PRD-BAK-01 age multi-recipient backup

#### [ ] 28.1 age 통합

##### [ ] 28.1.1 rage 라이브러리 의존

  - [ ] 1단계 중복검사: 기존 backup 코드
  - [ ] 2단계 Context7: age = "0.10" / rage
  - [ ] 3단계 구현: workspace dep
  - [ ] 4단계 simpler: feature 최소
  - [ ] 5단계 검증: cargo build
  - [ ] 6단계 [x]

##### [ ] 28.1.2 X25519 master + 비상 recipient 2

  - [ ] 1단계 중복검사: 기존 키 관리
  - [ ] 2단계 Context7: age::x25519::Identity
  - [ ] 3단계 구현: encrypt with [master_recipient, agent_master, paperkey]
  - [ ] 4단계 simpler: recipients 목록 한곳
  - [ ] 5단계 검증: 3 키 모두 복호 가능
  - [ ] 6단계 [x]

##### [ ] 28.1.3 xgram backup create / restore CLI

  - [ ] 1단계 중복검사: clap
  - [ ] 2단계 Context7: clap derive
  - [ ] 3단계 구현: BackupCmd
  - [ ] 4단계 simpler: 단일
  - [ ] 5단계 검증: --help + 라운드트립
  - [ ] 6단계 [x]

##### [ ] 28.1.4 PQ readiness — hybrid wrap 인터페이스

  - [ ] 1단계 중복검사: hybrid 코드
  - [ ] 2단계 Context7: NIST FIPS 203/204
  - [ ] 3단계 구현: WrapEngine trait + age impl만
  - [ ] 4단계 simpler: trait 한곳
  - [ ] 5단계 검증: 인터페이스 모킹 테스트
  - [ ] 6단계 [x]

---

## Phase 2.3 Tauri R/W (Vite + Solid + Plugins)

### [ ] 29. PRD-TAURI-01 Vite + Solid.js + TypeScript 마이그레이션

#### [ ] 29.1 빌드 시스템 교체

##### [ ] 29.1.1 ui/ 디렉토리 정적 HTML 폐기

  - [ ] 1단계 중복검사: 기존 ui/ 구조
  - [ ] 2단계 Context7: Vite 5
  - [ ] 3단계 구현: ui/legacy 보관 + 신규 ui/app
  - [ ] 4단계 simpler: 디렉토리 한곳
  - [ ] 5단계 검증: 기존 빌드 비교
  - [ ] 6단계 [x]

##### [ ] 29.1.2 npm init vite + solid-ts 템플릿

  - [ ] 1단계 중복검사: package.json
  - [ ] 2단계 Context7: create-vite solid-ts
  - [ ] 3단계 구현: ui/app 초기화
  - [ ] 4단계 simpler: 의존 최소
  - [ ] 5단계 검증: npm run dev
  - [ ] 6단계 [x]

##### [ ] 29.1.3 tauri.conf.json devUrl/frontendDist 매핑

  - [ ] 1단계 중복결사: tauri.conf
  - [ ] 2단계 Context7: tauri 2 build
  - [ ] 3단계 구현: devUrl=http://localhost:5173, frontendDist=ui/app/dist
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: tauri dev 실행
  - [ ] 6단계 [x]

##### [ ] 29.1.4 빌드 결과 < 200KB 확인

  - [ ] 1단계 중복검사: bundle 분석
  - [ ] 2단계 Context7: vite-bundle-analyzer
  - [ ] 3단계 구현: bundle 측정 스크립트
  - [ ] 4단계 simpler: 한 명령
  - [ ] 5단계 검증: 사이즈 보고
  - [ ] 6단계 [x]

### [ ] 30. PRD-TAURI-02 Stronghold + Channel API + 7 plugins

#### [ ] 30.1 plugins 통합

##### [ ] 30.1.1 plugin-stronghold

  - [ ] 1단계 중복검사: 기존 stronghold 사용
  - [ ] 2단계 Context7: tauri-plugin-stronghold
  - [ ] 3단계 구현: snapshot path 설정
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: store/load 라운드트립
  - [ ] 6단계 [x]

##### [ ] 30.1.2 plugin-dialog/clipboard-manager/notification

  - [ ] 1단계 중복검사: 기존 사용
  - [ ] 2단계 Context7: tauri-plugin-* 2.x
  - [ ] 3단계 구현: 등록
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: dialog 호출 동작
  - [ ] 6단계 [x]

##### [ ] 30.1.3 plugin-updater/store/global-shortcut/single-instance

  - [ ] 1단계 중복검사: 기존 사용
  - [ ] 2단계 Context7: tauri-plugin-* 2.x
  - [ ] 3단계 구현: 등록 + 권한 capabilities
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: 각 plugin 호출 1회
  - [ ] 6단계 [x]

##### [ ] 30.1.4 Channel API — pending 큐 스트림

  - [ ] 1단계 중복검사: ipc::Channel
  - [ ] 2단계 Context7: tauri::ipc::Channel
  - [ ] 3단계 구현: Rust → front Channel<PendingEvent>
  - [ ] 4단계 simpler: 한 모듈
  - [ ] 5단계 검증: front 수신
  - [ ] 6단계 [x]

### [ ] 31. PRD-TAURI-03 Vault Pending approve/deny UI

#### [ ] 31.1 UI 액션

##### [ ] 31.1.1 Pending 리스트 뷰 (가상 리스트)

  - [ ] 1단계 중복검사: 기존 pending 코드
  - [ ] 2단계 Context7: @tanstack/solid-virtual
  - [ ] 3단계 구현: PendingList component
  - [ ] 4단계 simpler: signal 단일
  - [ ] 5단계 검증: 1000 row 스크롤
  - [ ] 6단계 [x]

##### [ ] 31.1.2 approve invoke + dialog confirm

  - [ ] 1단계 중복검사: approve 함수
  - [ ] 2단계 Context7: invoke + plugin-dialog
  - [ ] 3단계 구현: invoke("vault_approve", id)
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 승인 후 row 제거
  - [ ] 6단계 [x]

##### [ ] 31.1.3 deny invoke + 사유 입력

  - [ ] 1단계 중복검사: deny 함수
  - [ ] 2단계 Context7: invoke
  - [ ] 3단계 구현: invoke("vault_deny", {id, reason})
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 거부 후 audit
  - [ ] 6단계 [x]

##### [ ] 31.1.4 mfa 정책 시 OS biometric / master pw 재입력

  - [ ] 1단계 중복검사: mfa hub
  - [ ] 2단계 Context7: webauthn-rs / tauri-plugin-biometric
  - [ ] 3단계 구현: mfa 정책 매핑
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 시뮬 mfa
  - [ ] 6단계 [x]

### [ ] 32. PRD-TAURI-04 Search across L0~L4

#### [ ] 32.1 검색 UI

##### [ ] 32.1.1 디바운스 입력 (300ms)

  - [ ] 1단계 중복검사: debounce util
  - [ ] 2단계 Context7: @solid-primitives/debounce
  - [ ] 3단계 구현: input → debounced signal
  - [ ] 4단계 simpler: 한 hook
  - [ ] 5단계 검증: 입력 1회당 invoke 1회
  - [ ] 6단계 [x]

##### [ ] 32.1.2 invoke memory_search(query, layers)

  - [ ] 1단계 중복검사: memory_search Rust
  - [ ] 2단계 Context7: tauri command
  - [ ] 3단계 구현: 명령 등록 + 응답 매핑
  - [ ] 4단계 simpler: 한 모듈
  - [ ] 5단계 검증: 결과 출력
  - [ ] 6단계 [x]

##### [ ] 32.1.3 layer 필터 (L0~L4 체크박스)

  - [ ] 1단계 중복검사: layer enum
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 체크박스 + signal
  - [ ] 4단계 simpler: enum 한곳
  - [ ] 5단계 검증: 필터링 동작
  - [ ] 6단계 [x]

##### [ ] 32.1.4 결과 가상 리스트

  - [ ] 1단계 중복검사: virtual
  - [ ] 2단계 Context7: @tanstack/solid-virtual
  - [ ] 3단계 구현: SearchResults component
  - [ ] 4단계 simpler: signal 단일
  - [ ] 5단계 검증: 1000 row 스크롤
  - [ ] 6단계 [x]

### [ ] 33. PRD-TAURI-05 Peer add UI + fingerprint 확인

#### [ ] 33.1 Peer R/W

##### [ ] 33.1.1 Peer 목록 뷰

  - [ ] 1단계 중복검사: peers 명령
  - [ ] 2단계 Context7: tauri command
  - [ ] 3단계 구현: PeersList component
  - [ ] 4단계 simpler: signal
  - [ ] 5단계 검증: invoke 결과 표시
  - [ ] 6단계 [x]

##### [ ] 33.1.2 add invoke (alias/address/pubkey)

  - [ ] 1단계 중복검사: peer-add 명령
  - [ ] 2단계 Context7: tauri invoke
  - [ ] 3단계 구현: form + invoke
  - [ ] 4단계 simpler: 한 form
  - [ ] 5단계 검증: peer row 생성
  - [ ] 6단계 [x]

##### [ ] 33.1.3 fingerprint 표시 + dialog confirm

  - [ ] 1단계 중복검사: pubkey hex
  - [ ] 2단계 Context7: plugin-dialog
  - [ ] 3단계 구현: SHA256 short fingerprint 표시 후 confirm
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 확인 절차
  - [ ] 6단계 [x]

##### [ ] 33.1.4 machine whitelist 적용

  - [ ] 1단계 중복검사: ACL
  - [ ] 2단계 Context7: 기존 ACL 코드
  - [ ] 3단계 구현: machine_id 입력 + ACL row
  - [ ] 4단계 simpler: form 한곳
  - [ ] 5단계 검증: ACL 정합성
  - [ ] 6단계 [x]

### [ ] 34. PRD-TAURI-06 Vault reveal + clipboard auto-clear

#### [ ] 34.1 reveal 액션

##### [ ] 34.1.1 reveal 토글 — 30s 마스킹 해제

  - [ ] 1단계 중복검사: reveal 코드
  - [ ] 2단계 Context7: solid signal + setTimeout
  - [ ] 3단계 구현: revealUntil signal
  - [ ] 4단계 simpler: 한 hook
  - [ ] 5단계 검증: 30s 후 자동 마스킹
  - [ ] 6단계 [x]

##### [ ] 34.1.2 clipboard 30s 자동 클리어

  - [ ] 1단계 중복검사: clipboard
  - [ ] 2단계 Context7: tauri-plugin-clipboard-manager
  - [ ] 3단계 구현: 복사 후 30s setTimeout → clear
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: 30s 후 빈 clipboard
  - [ ] 6단계 [x]

##### [ ] 34.1.3 vault 평문 invoke 응답 직접 X (Stronghold 토큰)

  - [ ] 1단계 중복검사: 기존 vault_get
  - [ ] 2단계 Context7: stronghold ephemeral
  - [ ] 3단계 구현: 단발 토큰만 webview, plaintext stronghold 임시 저장
  - [ ] 4단계 simpler: 단일 함수
  - [ ] 5단계 검증: invoke 응답에 plaintext 없음 확인
  - [ ] 6단계 [x]

##### [ ] 34.1.4 zeroize 보장

  - [ ] 1단계 중복검사: zeroize
  - [ ] 2단계 Context7: zeroize crate
  - [ ] 3단계 구현: drop 시 wipe
  - [ ] 4단계 simpler: 한 type
  - [ ] 5단계 검증: 메모리 dump 시뮬
  - [ ] 6단계 [x]

### [ ] 35. PRD-TAURI-07 Payment 한도 변경 + MFA 재인증

#### [ ] 35.1 한도 R/W

##### [ ] 35.1.1 한도 view + 편집 form

  - [ ] 1단계 중복검사: limits 코드
  - [ ] 2단계 Context7: solid form
  - [ ] 3단계 구현: PaymentLimits component
  - [ ] 4단계 simpler: 한 form
  - [ ] 5단계 검증: 표시 정상
  - [ ] 6단계 [x]

##### [ ] 35.1.2 invoke set_payment_limit

  - [ ] 1단계 중복검사: limit 명령
  - [ ] 2단계 Context7: tauri invoke
  - [ ] 3단계 구현: 명령 등록
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: DB row 변경
  - [ ] 6단계 [x]

##### [ ] 35.1.3 mfa 재인증 (master pw + biometric)

  - [ ] 1단계 중복검사: mfa hub
  - [ ] 2단계 Context7: webauthn-rs (PRD-MFA-02)
  - [ ] 3단계 구현: 한도 변경 시 mfa 강제
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: mfa 미통과 시 거부
  - [ ] 6단계 [x]

##### [ ] 35.1.4 hash chain LIMIT_CHANGE 이벤트 기록

  - [ ] 1단계 중복검사: audit
  - [ ] 2단계 Context7: PRD-AUDIT-01
  - [ ] 3단계 구현: 한도 변경 시 audit row
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: row 존재
  - [ ] 6단계 [x]

### [ ] 36. PRD-TAURI-08 자동 업데이트 (minisign)

#### [ ] 36.1 updater

##### [ ] 36.1.1 plugin-updater 활성

  - [ ] 1단계 중복검사: updater
  - [ ] 2단계 Context7: tauri-plugin-updater
  - [ ] 3단계 구현: endpoint + minisign pubkey
  - [ ] 4단계 simpler: config 한곳
  - [ ] 5단계 검증: tauri 빌드
  - [ ] 6단계 [x]

##### [ ] 36.1.2 GitHub Releases 메니페스트 생성 스크립트

  - [ ] 1단계 중복검사: scripts/
  - [ ] 2단계 Context7: gh release
  - [ ] 3단계 구현: latest.json 생성
  - [ ] 4단계 simpler: 한 스크립트
  - [ ] 5단계 검증: dry-run
  - [ ] 6단계 [x]

##### [ ] 36.1.3 minisign 서명 키 관리 docs

  - [ ] 1단계 중복검사: docs
  - [ ] 2단계 Context7: minisign
  - [ ] 3단계 구현: docs/release-signing.md
  - [ ] 4단계 simpler: 한 문서
  - [ ] 5단계 검증: 마스터 검토
  - [ ] 6단계 [x]

##### [ ] 36.1.4 update prompt UI

  - [ ] 1단계 중복검사: dialog
  - [ ] 2단계 Context7: solid component
  - [ ] 3단계 구현: UpdatePrompt component
  - [ ] 4단계 simpler: 한 component
  - [ ] 5단계 검증: 시뮬 호출
  - [ ] 6단계 [x]

### [ ] 37. PRD-TAURI-09 i18n (KR/EN)

#### [ ] 37.1 다국어

##### [ ] 37.1.1 @solid-primitives/i18n 설정

  - [ ] 1단계 중복검사: 기존 lang
  - [ ] 2단계 Context7: solid-primitives i18n
  - [ ] 3단계 구현: i18n provider + signals
  - [ ] 4단계 simpler: 한 모듈
  - [ ] 5단계 검증: 언어 전환 동작
  - [ ] 6단계 [x]

##### [ ] 37.1.2 ko.json / en.json 메시지 카탈로그

  - [ ] 1단계 중복검사: 메시지 추출
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 두 카탈로그 한곳
  - [ ] 4단계 simpler: 키 단일
  - [ ] 5단계 검증: 누락 키 검사
  - [ ] 6단계 [x]

##### [ ] 37.1.3 OS locale 자동 감지

  - [ ] 1단계 중복검사: locale
  - [ ] 2단계 Context7: tauri get_locale
  - [ ] 3단계 구현: 기동 시 OS locale → KR/EN 매핑
  - [ ] 4단계 simpler: 한 함수
  - [ ] 5단계 검증: env LANG 영향
  - [ ] 6단계 [x]

##### [ ] 37.1.4 사용자 override (plugin-store 저장)

  - [ ] 1단계 중복검사: store
  - [ ] 2단계 Context7: tauri-plugin-store
  - [ ] 3단계 구현: locale.preference 저장
  - [ ] 4단계 simpler: 한곳
  - [ ] 5단계 검증: 재시작 후 유지
  - [ ] 6단계 [x]

---

## 9. 최종 재검증 (모든 [x] 후)

### [ ] 38. 전체 재검증 — 직접 실행

#### [ ] 38.1 cargo workspace 빌드·테스트·clippy

##### [ ] 38.1.1 cargo build --workspace --release

  - [ ] 1단계 중복검사: 빌드 캐시
  - [ ] 2단계 Context7: cargo
  - [ ] 3단계 구현: 명령 실행
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 0 error
  - [ ] 6단계 [x]

##### [ ] 38.1.2 cargo test --workspace

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 명령 실행
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 0 fail
  - [ ] 6단계 [x]

##### [ ] 38.1.3 cargo clippy --workspace -- -D warnings

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 명령 실행
  - [ ] 4단계 simpler: warning 0 까지 fix
  - [ ] 5단계 검증: 0 warning
  - [ ] 6단계 [x]

##### [ ] 38.1.4 통합 테스트 ≥ 80 확인

  - [ ] 1단계 중복검사: cargo test 카운트
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 카운트 보고
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: ≥ 80
  - [ ] 6단계 [x]

#### [ ] 38.2 시나리오별 e2e 재실행

##### [ ] 38.2.1 머신 A→B Nostr cross-network round-trip

  - [ ] 1단계 중복검사: 픽스처
  - [ ] 2단계 Context7: nostr-relay-builder
  - [ ] 3단계 구현: e2e 실행
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 메시지 수신 확인
  - [ ] 6단계 [x]

##### [ ] 38.2.2 payment intent submit→confirm (Base testnet)

  - [ ] 1단계 중복검사: testnet 픽스처
  - [ ] 2단계 Context7: alloy
  - [ ] 3단계 구현: 실 송신
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: confirmed
  - [ ] 6단계 [x]

##### [ ] 38.2.3 vault rotate-kek round-trip + audit chain 무결성

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 실 회전
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: verify 통과
  - [ ] 6단계 [x]

##### [ ] 38.2.4 audit verify fault injection (직접 row 삭제)

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 실 DB 변조
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 즉시 검출
  - [ ] 6단계 [x]

#### [ ] 38.3 v0.2.0 GA 태깅

##### [ ] 38.3.1 version.json + package.json 동기화

  - [ ] 1단계 중복검사: version 동기화 스크립트
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: 0.2.0 으로 업데이트
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: jq 검사
  - [ ] 6단계 [x]

##### [ ] 38.3.2 CHANGELOG 갱신

  - [ ] 1단계 중복검사: CHANGELOG.md
  - [ ] 2단계 Context7: keepachangelog
  - [ ] 3단계 구현: v0.2.0 섹션
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 마스터 검토 가능 형식
  - [ ] 6단계 [x]

##### [ ] 38.3.3 git tag v0.2.0 + push

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: tag 생성
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: GitHub release 생성
  - [ ] 6단계 [x]

##### [ ] 38.3.4 디스코드 GA 보고

  - [ ] 1단계 중복검사: -
  - [ ] 2단계 Context7: -
  - [ ] 3단계 구현: webhook 발송
  - [ ] 4단계 simpler: -
  - [ ] 5단계 검증: 응답 200
  - [ ] 6단계 [x]
