# 5번 재작업의 진짜 뿌리 — 코드 확정 (2026-06-24)

> 추측 아님. 모든 주장에 file:line 증거.

## 한 줄
세션↔신원 바인딩이 **alias 문자열 매칭 + write-once**다. 마스터가 설계한
**keypair(공개키) 앵커 + 재시작마다 재바인딩**이 아니다. → 재시작/이름불일치 때마다
바인딩이 stale → 라우팅 실패 → "통신 안 됨" → 재작업. 이 사이클이 5번 반복.

## 증거 1 — auto-seed 가 alias 문자열로 매칭 (keypair 아님)
`daemon.rs:2119-2124`:
```
let sid = format!("tmux:{sn}");                       // sn = tmux 세션 이름
UPDATE peers SET session_identifier = ?1
  WHERE alias = ?2 AND (session_identifier IS NULL OR session_identifier = '')
```
- 바인딩 키 = `alias = <tmux 세션이름에서 파생한 alias>`. **공개키 아님.**
- peer.alias=`gemini` 인데 tmux 세션=`aoe_starian-gemini_134fd5d8` → alias 불일치 →
  **auto-bind 무발동.** 그래서 gemini 는 매번 "수동설정" 필요했음(핸드오프 기록과 일치).
- 마스터 설계 정면 위배: "동일성 기준 = 공개키 앵커, 단순 파싱 금지"
  (roster-identity-foundation-handoff 메모리).

## 증거 2 — write-once (재시작 때 stale 안 고침)
`daemon.rs:2122`: `... session_identifier IS NULL OR = ''` 조건.
- 한 번 set 되면 **다시 안 씀.** 세션 재시작→새 tmux 이름이어도 옛 값 유지(stale).
- 결과: 재시작 후 메시지가 죽은 세션으로 라우팅 → 도달 안 됨.
- 핸드오프 root 증상 #1·#2·#4 = 이것.

## 증거 3 — 다른 write 경로도 전부 alias-keyed
- `mcp_serve.rs:2035` register_subagent: `UPDATE ... WHERE alias = ?2` — 에이전트가
  **자발적으로** session_identifier 넘길 때만. 데몬 자동 아님.
- `daemon.rs:1115` rc.244 zero-touch: addr+gui 만 갱신, **session_identifier 안 건드림.**
- 공개키 앵커는 `identity_registry.rs check_name_available` 의 **dedup 판정**에만 있고,
  실제 session_identifier **WRITE 에는 안 쓰임.** → 앵커가 바인딩에 미적용.

## 오늘(rc.371) 작업의 위치
- 발신자별 서명 + tmux 주입 = **진짜로 동작**(seoul↔gemini 실왕복 검증됨).
- 그러나 **이 root 와 직교(orthogonal).** 오늘 테스트는 이전 세션이 **수동 바인딩**
  해둔 발판 위에서 돈 것. 재시작하면 또 깨짐 → 그래서 band-aid 위험.

## 똑바로 된 fix (root)
1. **바인딩을 공개키 앵커로**: live tmux 세션 ↔ peer 매칭을 alias 문자열이 아니라
   그 세션 에이전트의 신원(pubkey)로. (세션이름≠alias 여도 바인딩.)
2. **재시작마다 재바인딩**: auto-seed tick 이 `IS NULL` 조건 빼고, **같은 pubkey 의
   현재 live tmux 로 session_identifier 를 매 tick 갱신**(stale 덮어쓰기). 단 사용자
   UI override 는 별도 보호 플래그로.
3. 기존 **P1 plan(커밋 3c72e97, migration 0065_identity_aliases) 에 정렬** — 새 설계
   만들지 말고 그 IdentityStore TDD 태스크로.

## 검증 게이트 (band-aid 재발 방지 — 절대)
happy-path 금지. **durable property 를 독립 LLM(gemini/codex)이 판정**:
- (A) gemini 세션 kill→새 session_id 로 respawn→**수동 재바인딩 0** 상태에서 seoul 도달?
- (B) cross-machine seoul↔zalman 실왕복?
둘 다 통과해야 "고침". 하나라도 수동 개입 필요하면 = 여전히 깨진 것.
Claude 자가검증 금지(마스터 룰).
