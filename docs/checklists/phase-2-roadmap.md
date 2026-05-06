# Phase 2 Roadmap — OpenXgram

> **상태: ✅ COMPLETED — v0.2.0-alpha.1 (2026-05-04 KST) Phase 2 GA 출하 완료.**
> 실제 출하된 변경은 [CHANGELOG.md](../../CHANGELOG.md) `[0.2.0]` 섹션을 참조하세요.
> 이 문서는 Phase 2 계획·범위 산정 원본 roadmap이며, 역사적 참조용으로 보존됩니다.
>
> Phase 1 (v0.1.0 GA) 머지 시점: 2026-05-04 KST. PR #45~#76 (32 PR) 누적.
> 이 문서는 Phase 2 의 작업 단위·우선순위·검증 기준을 정리한다.

## Phase 1 → Phase 2 경계

Phase 1 GA (v0.1.0) 에서 외부 사용 가능 안정 표면을 확보:
- 5층 메모리 (L0~L4) + L3→L4 자동 도출
- Vault 4단계 보안 (ACL · 일일한도 · 감사로그 · auto/confirm/mfa)
- MCP stdio + HTTP Bearer 인증
- Tailscale 자동 bind
- 비파괴 backup + systemd .timer + restore --merge
- 9단계 wizard
- doctor 10 체크
- shell completion + JSON 로그
- Tauri 데스크톱 6 탭 (Status/Sessions/Vault/Pending/Peers/Payments)
- Peer registry + alias-based send
- USDC payment intent 인프라

Phase 2 는 **외부 통합·자율성·결제** 에 집중.

## P2.0 — 통합 강화 (1~2주)

### 2.0.1 alloy/ethers RPC 통합 (payment.submit)
- **목표**: PaymentStore 가 실제 on-chain 트랜잭션 제출.
- **작업**:
  - `crates/openxgram-payment` 에 alloy 0.x dep 추가
  - master keypair (secp256k1) → alloy `LocalSigner` 변환
  - USDC `transfer(address,uint256)` ABI 인코딩
  - EIP-1559 트랜잭션 빌드 (gas estimate + fee suggestion)
  - RPC POST → tx_hash 반환
  - PaymentStore.submit(intent) 메서드 추가
- **검증**: Base testnet 통합 테스트 (Tenderly fork 또는 Anvil 로컬)
- **위험**: 실 자금 다루므로 신중. 마스터 승인 후 mainnet 전환.

### 2.0.2 inbound webhook → peer.touch 자동
- **목표**: peer 가 보낸 메시지 수신 시 last_seen 자동 갱신.
- **작업**:
  - transport `/v1/message` 핸들러가 envelope.from → public_key 매핑 → peer.touch(alias)
  - PeerStore::touch_by_public_key(pk_hex) 추가
- **검증**: peer A → peer B send 후 B 의 PeerStore 에서 A.last_seen 확인
- **포함**: 서명 검증 (envelope.signature_hex 가 envelope.from 의 master_public_key 와 일치)

### 2.0.3 batch send (1:N peer)
- **목표**: 여러 peer 에 동시 전송, 실패 격리.
- **작업**:
  - `xgram peer broadcast --aliases a,b,c --body "..."` — N개 동시 send
  - 각 결과 (성공/실패) 보고
  - `--exclude` 플래그로 특정 peer 제외
- **검증**: 3 peer 중 1개 down → 다른 2개는 성공 + 보고

## P2.1 — XMTP 통합 (3~5일)

### 2.1.1 XMTP SDK 평가
- xmtp_v3 0.x 또는 xmtp-rs 의존
- secp256k1 마스터 키 → XMTP signing key 매핑 (EIP-191/EIP-712)
- 첫 prototype: send_message + read_messages

### 2.1.2 xmtp:// peer scheme 처리
- peer.address = "xmtp://0xRecipient" 형식 인식
- peer_send.rs 가 scheme 분기 → XMTP route
- XmtpClient lazy-init (master 키 1회 로드)

### 2.1.3 inbound XMTP polling
- daemon 측 XMTP message stream subscribe
- envelope 으로 변환 후 transport `/v1/message` 와 동일 경로
- PeerStore.touch + L0 messages 저장

### 2.1.4 검증 시나리오
- 머신 A (XMTP wallet) → 머신 B (XMTP wallet) cross-network 메시지
- B 의 message store 에 도착 + ECDSA 검증 통과
- A 의 PeerStore.touch(B) 자동 갱신

## P2.2 — Tauri GUI 확장 (1주)

### 2.2.1 R/W 액션
- Sessions 탭에 "새 session" 버튼 → modal → xgram session new
- Vault 탭에 "새 자격증명" 버튼 → modal → xgram vault set
- Pending 탭에 approve/deny 버튼 → modal confirm

### 2.2.2 Search/Recall
- Sessions 탭에 검색 박스 → xgram session recall --query
- 결과를 거리순 카드로 표시

### 2.2.3 자동 새로고침
- 30초 polling (각 탭별 활성 시)
- pending 탭은 더 짧은 주기 (5초) — 마스터 즉시 응답 친화

### 2.2.4 자동 업데이트 (Tauri updater)
- GitHub Releases 엔드포인트 통합
- 마스터가 새 버전 푸시하면 데스크톱 알림 → 1-click 업데이트

## P2.3 — 운영성 강화 (1주)

### 2.3.1 Prometheus exporter
- daemon 에 `/metrics` 엔드포인트 (text-based)
- 노출 메트릭: vault_audit_total, pending_count, message_count, embedder_mode, tailscale_state
- 레이블: agent, action, kind

### 2.3.2 OpenTelemetry trace
- tracing crate → otel exporter (tonic gRPC)
- HTTP/MCP 요청 trace ID 전파

### 2.3.3 retention 정책
- L0 messages 90일 후 episode summary 만 남기기
- vault_audit 1년 보존
- 자동 archival (cold storage 옵션)

## P2.4 — 신뢰·감사 (1주)

### 2.4.1 manifest signing chain
- install-manifest 위에 audit log signing chain (각 변경이 master 서명 + 이전 hash 참조)
- tamper detection

### 2.4.2 secret rotation
- master 키페어 회전 — 새 키 + 마이그레이션 도구
- vault 재암호화 (zero-downtime)

## 측정·완료 기준

### 정량
- Phase 2 끝 v0.2.0 GA 태깅
- 신규 통합 테스트 ≥ 50건
- workspace clippy 0 warnings 유지
- CI 시간 5분 이내 (이번 PR #71 기반 병렬 효과)

### 정성
- 마스터의 1개 머신 + 1개 클라우드 머신 사이 cross-network 메시지 round-trip 성공 (XMTP 또는 Tailscale)
- payment intent submit → confirm 1회 성공 (Base testnet)
- Tauri 데스크톱 앱 daily-driver 가능 (Sessions create / Vault set / Pending approve 모두 GUI 만으로)

## 우선순위 결정 기준

마스터 결정 가이드:
1. **2.0.x 통합 강화** — Phase 1 의 missing wiring 채움. 작업당 0.5~1일.
2. **2.1.x XMTP** — cross-network 진짜 가치. 어느 시점엔 필수.
3. **2.2.x Tauri R/W** — 사용자 경험. CLI 사용자에겐 후순위.
4. **2.3.x 운영성** — 실 운영 단계 진입 시 필요.
5. **2.4.x 신뢰** — 감사·보안 강화. 상시 가능.

기본 진행 순서: **2.0.1 → 2.0.2 → 2.1.x → 2.2.x → 2.3.x → 2.4.x**

마스터 별도 지시 없으면 위 순서대로 자율 진행.
