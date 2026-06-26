# 3-4 cross-daemon CRUD = A2A signed command — 설계 (합의용 v0.1)

> 작성: zalman/claude (aoe_openxgram-claude_6f47c74f) · 2026-06-27
> 정본: `.handoff/KAKAOTALK-MESSENGER-SPEC.md` §2 액션 + §2.24 "모든 기능 API로 외부 제어"
> 원칙: ACP=사람↔에이전트, **A2A=데몬↔데몬**. HTTP 프록시/fallback 금지. 기존 A2A 네이티브 우선.
> 교차검증: seoul(독립검토) + hermes(리뷰). 브랜치 `feat/a2a-cross-daemon-crud`.

## 1. 갭 최종 분류 (seoul transport 독립확인 반영)

| 갭 | 상태 | 처리 |
|----|------|------|
| GAP-3 anti-replay | **가짜 갭** | transport 네이티브 재사용. 액션 envelope 이 `nonce` 를 **반드시 SET**(Option이라 채움). replay.rs (agent,nonce)+90초 윈도우. 재구현 금지. |
| GAP-5 per-sender auth | **거의 해결** | envelope 서명검증 존재(`verify_with_pubkey`, daemon.rs:1183). 핸들러가 payload `from_agent`(위조가능) 아니라 **검증된 서명자**로 인가. 서명로직 신규 금지. |
| GAP-6 action 스키마 | **실작업** | payload 안 structured action JSON + `envelope_type="a2a_command"`. |
| GAP-1 원격 행 DB mutation | **실작업** | 소유 데몬 수신 핸들러: display_name/role/token_price/sample/balance UPDATE. |
| GAP-2 원격 kill/restart/spawn | **실작업** | 소유 데몬 수신 핸들러: 로컬 ACP/tmux 제어 재사용(kill_tmux_session 등). |
| GAP-4 ACL | **실작업** | 검증된 envelope sender 기준 + **소유 데몬만 자기 행 mutate 허용**. |

## 2. 인가 토대 (GAP-4/5 핵심) — seoul 독립확인 정밀화 (CONFIRMED)

진실의 신원 = **수신측 peers 테이블에 등록된 pubkey 로 서명검증 통과한 발행자**.

코드 앵커 (확인 완료):
- `verify_with_pubkey` = `openxgram-keystore/src/keypair.rs:114` (secp256k1 ECDSA).
- `daemon.rs:1144-1147`: `verified = match peer_opt { Some(p) => verify_with_pubkey(&p.public_key_hex, payload, sig).is_ok(), None => false }`.
  → **등록 peer 의 저장된 pubkey 로 대조**하므로 "claim pubkey + 자기서명" 위장 불가(그 키가 수신측에 이미 등록돼야 함).
- ⚠️ `daemon.rs:1154`: `alias = peer_opt.map(p.alias).unwrap_or(env.from)` — peer 매칭 실패 시 **env.from 폴백 = UNVERIFIED 경로**.

a2a_command 인가 게이트 — **3중 ALL pass 필수** (하나라도 실패 → denied):
1. `verified == true` (peer_opt Some + 서명통과). `peer_opt.is_none()` 또는 서명실패 → 거부. **env.from 폴백 경로 전면 거부.**
2. 발행자 = 매칭된 peer 신원(eth/alias). env.from 폴백 사용 금지.
3. 발행자 eth ∈ **신뢰 allowlist** (default-deny). 서명유효만으론 부족 — allowlist 멤버여야.

추가 게이트:
4. 수신 데몬은 **자기 소유 행(origin_machine == self machine)** 에 대해서만 실행. 남의 소유 행 명령 → denied.
5. denied 는 반드시 `a2a_command_result{ok:false, error:"denied:<사유>"}` 회신 — 조용히 무시 금지(fallback 금지 원칙).

### ACL allowlist (v1 — seoul 확정)
- **default-deny**. 소유 데몬이 신뢰하는 발행자 데몬 eth 목록(설정·동적).
- 기본값 = 마스터 fleet 데몬(seoul / zalman). 확장 가능.
- 발행자 = **요청 데몬 신원**(마스터가 seoul GUI 에서 zalman 행 액션 → seoul 데몬이 서명발행 → zalman ACL 이 seoul 을 신뢰).
- (c) "전체 verified peer 허용"은 **금지** — 아무 verified peer 가 세션 kill 가능하면 위험.

## 3. Action 스키마 (GAP-6) — payload JSON

Envelope 구조는 **안 건드림**. `envelope_type` 에 신규 값 `"a2a_command"` 추가, payload_hex 디코드 시 아래 JSON.

```json
{
  "v": 1,
  "command_id": "<uuid>",
  "action": "set_token_price | set_sample | set_role | set_display_name | wallet_charge | wallet_transfer | kill | restart | spawn",
  "target_alias": "<소유 데몬의 행 alias>",
  "args": { /* action 별 필드 */ },
  "issued_at": "<rfc3339>"
}
```

결과 회신 (`envelope_type="a2a_command_result"`):
```json
{ "v":1, "command_id":"<uuid>", "ok": true|false, "applied": {...}|null, "error": "<사유>"|null }
```

## 3.5 통합 지점 (코드 레벨 — 확인 완료)

process_inbound 에 envelope_type 분기 패턴 이미 확립:
- `daemon.rs:925` — `"ack"` 분기
- `daemon.rs:1017` — `"identity_update"` 분기 (원격 신호로 자기 로컬 display_name/role UPDATE — a2a_command 의 직접 선례)

→ **`"a2a_command"` 분기를 daemon.rs:1014 근처(identity_update 인접)에 추가.** ack/identity_update 처럼 inbox 저장·tmux inject skip 후 continue.

### ⚠️ 현존 취약점 (3-4 와 직결 — seoul 보고함)
기존 `identity_update` 분기(daemon.rs:1017-1056)는 **인가 게이트가 전혀 없음**:
- `verified` 플래그 미확인 (서명검증 결과 무시)
- allowlist·origin_machine 소유권 체크 없음
- → **누구든 identity_update envelope 으로 임의 alias 의 display_name/role 원격 변경 가능** (현존 취약).

함의: GAP-4(ACL)는 신규 기능이 아니라 **현존 위험의 패치**이기도 함. a2a_command 는 처음부터 3중 게이트로 구현하고, identity_update 도 같은 게이트로 보강 검토(별도 합의 — 3-4 v1 범위 밖일 수 있음, seoul 판단).

## 4. 흐름

```
GUI/CLI 액션 (원격 행)
  → ownership 판별: is_remote_homed_peer(target) == true?
      → 로컬 행이면 기존 경로(직접 DB/ACP) 유지 — 3-4 무관
      → 원격이면 ↓
  → A2A command Envelope 작성 (nonce SET, sender 서명, payload=action JSON, type=a2a_command)
  → 소유 데몬으로 전송 (peer_send 의 send_envelope 재사용 — HTTP 프록시 아님, 서명 envelope)
  → 소유 데몬 process_inbound: type=a2a_command 분기
      → verify_with_pubkey (GAP-5) → 검증된 sender
      → ACL 체크 (GAP-4): 자기 소유 행 + sender 권한
      → 로컬 실행 (GAP-1 DB UPDATE / GAP-2 ACP·tmux)
      → a2a_command_result Envelope 회신
  → 발신 데몬: result 수신 → GUI 상태 반영
```

## 5. 검증 기준 (seoul 라이브 검증 대상)

- 로컬 행: 기존 경로 그대로 (회귀 없음).
- 원격 행: **local-only 로 수정 안 됨** (발신 데몬 DB 직접 변경 금지) + **소유 데몬 DB/상태 실제 변경**.
- 위조 from_agent: 서명 불일치 → denied.
- replay: 같은 nonce 재전송 → reject.
- 권한 없는 sender: denied 회신.

## 6. 결정 확정 (seoul 2026-06-27)

- **Q1 ACL = CONFIRMED**: (c) 전체 verified peer 허용 금지. default-deny 신뢰 발행자 allowlist(설정·동적, 기본=마스터 fleet 데몬). 발행자=요청 데몬 신원. → §2 ACL 섹션 반영.
- **Q2 범위 = CONFIRMED**: wallet_charge/transfer 는 v1 제외(double-spend 원자성·idempotency 별도 phase). **v1 = 행메타(이름·역할·토큰단가·샘플) + 세션제어(kill/restart/spawn)** 만.
- **Q3 (hermes 확인 대기)**: daemon_gui.rs 액션 핸들러는 파일 하단 추가 → hermes 3-1 구간(14848~14916, 341-346) 비침범. hermes commit/push 후 그 base 에서 worktree 분기.

### v1 action 화이트리스트 (스키마 `action` 허용값)
`set_display_name`, `set_role`, `set_token_price`, `set_sample`, `kill`, `restart`, `spawn`.
(wallet_charge / wallet_transfer 는 v2 — 스키마에 두되 핸들러가 `denied:not_in_v1` 회신.)
