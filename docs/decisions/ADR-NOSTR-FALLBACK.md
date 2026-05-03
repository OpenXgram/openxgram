# ADR — http → nostr fallback policy

> 상태: accepted (2026-05-04 KST)
> 관련 PRD: PRD-NOSTR-13 (deferred 2.7.3 of Phase 2.1 checklist)

## 결정

`peer.address` 가 `http(s)://` 인 경우 send 가 실패하면, 다음 조건이 **모두** 충족되었을 때만 `nostr://` 보조 경로로 재시도한다.

1. `peer` row 에 `nostr_relay` 보조 필드가 명시 등록되어 있다.
2. 프로세스 환경변수 `XGRAM_PEER_FALLBACK_NOSTR=1` 가 설정되어 있다 (opt-in).
3. 1차 http 실패 사유가 명시 로그(INFO 레벨)로 남는다 (silent 금지).

위 3조건 중 하나라도 누락되면 fallback 없이 raise 한다.

## 절대 규칙 정합성

마스터의 절대 규칙 6개 중 "fallback 금지" 와 충돌해 보일 수 있으므로 명확히:

- "fallback 금지" 의 본의 = **silent fallback 금지**. 조용히 다른 경로로 빠져 사용자가 인지 못하는 상황을 막기 위함.
- 본 정책 = **명시적 opt-in + 명시 로그**. 사용자가 의도해서 활성화한 경우만 동작, 동작 시 모든 시도를 INFO 로그로 추적 가능.
- 따라서 silent 금지 규칙 위반 아님. 오히려 운영 가시성을 보장한다.

## 구현 메모

- `peer` 테이블에 `nostr_relay TEXT` 컬럼 추가 (nullable, 기본 NULL).
- `peer add` CLI 에 `--nostr-relay <ws_url>` 옵션 추가.
- `run_peer_send` 가 http 경로 실패 시 위 3조건 평가:
  - 통과: `tracing::info!(target: "openxgram_peer_send", "http 실패 — XGRAM_PEER_FALLBACK_NOSTR opt-in 으로 nostr 재시도")` 후 `send_via_nostr` 호출
  - 미통과: 즉시 raise (silent fallback 절대 금지)

## 향후

- 본 ADR 은 Phase 2.1 잔여 정책 결정. 실제 schema migration 은 후속 PR.
- nostr → http 역방향 fallback 은 별도 ADR 후 결정.
- mfa 정책 적용 등 자격증명 송신은 본 fallback 적용 제외 (보안 등급 변경 효과 → 마스터 별도 승인).
