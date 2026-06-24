# 정본 신원·등록·라우팅 모델 — 마스터 확정 스펙 (2026-06-24)

> 마스터와 다회 설계 확정. **이 문서가 정본.** `IDENTITY-ROSTER-MECHANISM-MAP.md`(원인 분석)·P1/P2 plan을 이 모델로 수렴시킨다.
> 핵심 원칙: **파싱·추측 폐기, 정확한 명칭으로만 작동.**

## 0. 한 줄 요약
사용자는 **이름(호출명) 하나**만 다룬다. 이름 ↔ terminalID를 **명시적으로 등록**하고, 모든 라우팅은 그 정확한 바인딩을 조회한다. 세션명 파싱·fuzzy 매칭 전면 폐기.

## A. 등록 (필수, 두 입구 · 공용 로직)
- 등록 안 한 것은 **부를 수 있는 대상으로 안 뜬다.**
- 입구 ①: **프론트 그리드** — tmux/acp 행에 이름 인라인 매칭(이름 ↔ terminalID).
- 입구 ②: **MCP `peer로 등록`** — 세션이 이름 입력해 명시 등록(필수 제공).
- 두 입구는 **같은 중앙 등록·중복 로직**을 탄다.

## B. MCP 자기식별 fallback (등록 안 한 세션이 통신 시도 시)
1. 먼저 **프로젝트 폴더(cwd) 기준**으로 기존 등록에서 매칭 → 있으면 그 신원으로 연결(terminalID·상태 갱신).
2. 없으면 **MCP 에러** → "peer 등록 먼저 하라" 강제.
- cwd는 *정확한 사실값* → 파싱 아님(원칙 일치).
- 조회 키 분리: **"나 누구"(자기식별)=cwd**, **"누구한테"(주소지정·중복)=이름**.

## C. 이름 = 신원·주소 (단일 네임스페이스)
- 종류 무관 **하나의 네임스페이스**. acp "star" 와 tmux "star" 동시 불가.
- 유일성 범위 = **내 네트워크 전체**(서울+잘만 등 합쳐서, 옵션 ② 확정).
- 중복(살아있는 다른 세션이 그 이름 보유) → **에러 알림** + **그 자리에서 인라인 변경** 또는 **충돌 세션 삭제** 안내(푸시).
- 죽은 동명 entry → 조용히 이어받기(갱신). 판별 = "살아있는 다른 terminalID가 쥐고 있나".

## D. 종류(kind) = 전달방식만
- **`acp` / `tmux` 둘뿐.** "어떻게 전달하나"만 의미.
- **`peer`·`agent`·`wsl`는 종류에서 제거** — peer/agent=provenance(내부만), wsl=환경 메타.
- "부를 수 있나"는 종류가 아니라 **등록됨 상태**로 표현.
- 현행 `is_peer?peer:has_agent?acp:tmux`(KakaoShell.tsx:321) 폐기 → kind = 전달방식 산출.

## E. 외부 경계 (다른 네트워크 에이전트 연결)
- 외부 에이전트는 **"내 네트워크에서 불릴 로컬 이름"으로 매핑 등록**.
- 로컬 이름 중복 시 알림 → 다른 로컬 이름 재지정 → 그 이름으로 사용.
- 외부의 두 에이전트가 동명 자칭해도 **keypair(숨김)로 구분** → 각각 다른 로컬 이름 매핑.

## F. 라우팅 = 이름 → terminalID 정확 조회
- 인바운드 A2A도 **결정적 resolver**(daemon_gui.rs:2876 = session_identifier 기반) 사용.
- **fuzzy substring resolver(notify.rs:567) 폐기.** 세션명 파싱 폐기.
- 결과: 회신이 항상 발신자 대화 세션으로.

## G. 그리드
- **등록된 것만 "부를 수 있는" 대상.** (이름 붙이려 터미널 목록은 보이되, 이름 없으면 비-호출 상태.)
- 컬럼: **이름(호출명, 인라인편집) · terminalID · 머신 · 종류(acp/tmux) · 상태(active/stopped/dead) · 액션(삭제/수정/종료/spawn)**.
- 중복 등록 시 에러 토스트 + 변경/삭제 안내.

## H. 지갑 (자리만 예약, 로직 나중)
- eth(공개키)=이미 지갑 주소(신원 앵커에 포함). 별도 작업 없음.
- 마이그레이션 0066에 **nullable 필드 예약**: `spending_limit`·`balance`·`earned`.
- 잔액 동기화·한도 집행·수익 추적 로직 + 지갑 UI = **마켓 연동 단계(별도)**. 지금 통신 그리드엔 안 넣음.

## I. 검증 (절대)
- **중간신호(빌드/db/테스트 통과)로 "됐다" 금지.**
- 실제 왕복: 보냄→올바른 세션 도달→답신→발신자 화면. **hermes/codex(다른 LLM)가 실동작 확인.**

## J. hermes 설계갭 반영 (2026-06-24 검토 — 토대에 포함)
1. **이름 유일성 권위자(split-brain 방지)**: 네트워크 전체 유일성은 단순 양쪽 DB unique로 부족 → 등록 레코드에 `origin_machine`·`updated_at`(monotonic)·`owner_public_key`를 담아 충돌 시 결정적 arbiter로 해소.
2. **죽은세션 이어받기 hijack 방지**: "살아있는 다른 terminalID" 판정에 **dead TTL·last_seen·health probe·lease 만료** 조건 명시. 상태 지연으로 산 세션을 죽었다 오판→탈취 금지.
3. **cwd fallback 다중후보**: cwd는 사실값이나 유일신원 아님(같은 폴더에 여러 세션). **후보 1개일 때만 자동 매칭, 2개+면 MCP 에러로 등록 선택 강제.**
4. **terminalID는 ephemeral**: tmux/ACP session id는 재시작 시 회전 → 장기 신원으로 쓰지 말 것. **stable=이름+keypair, terminalID=ephemeral binding** 분리. 라우팅은 이름→현재 terminalID 조회.
5. **kind 내부 route_type 보존**: UI는 acp/tmux만 보여도, 내부엔 `route_type`(acp-existing/acp-new/tmux/direct-portal)·`supports_prompt/screen_capture/ack` 같은 capability 보존(운영 디버깅용).
6. **외부 reply correlation**: 외부 alias→로컬 이름 매핑 시 `conversation_id`·`remote_public_key`·`remote_machine` 동반 저장 → 답신이 정확히 원 발신 네트워크로.
7. **migration기 명시 실패**: 미등록/미해결 대상은 조용히 fallback 금지 → **"unregistered/unroutable" 명시 실패 + UI/로그 노출.** (조용히 star로 보내는 fallback 금지.)
8. **ack 단계 분리**: 검증 ack를 `sent/db_saved/injected/submitted/processed/replied`로 분리 → "도착했는데 ack만 안 됨" vs "LLM 미도달" 구분.

## 임시 통신 채널 (p2p 정상화까지)
- p2p(신원·등록) 완성 전까지 **portal webhook**을 에이전트 간 임시 채널로 사용(마스터 지시 2026-06-24).
- 로컬: `POST http://localhost:9400/api/webhook/<session>:<win>` + `Authorization: Bearer <passcode>` + `{"command":"..."}`. terminalId = `<tmux세션>:<창index>` (예: `aoe_hermes-star_9ddbd0ed:0`).
- 외부(크로스머신): seoul portal `remote-proxy` → 상대 portal `/api/webhook/<상대terminalId>`.
- passcode = `~/.starian/portal-settings.json`(평문 노출 금지). **토대 완성 후 webhook 떼고 p2p로 전환.**

## 구현 슬라이스 (순서)
1. **migration 0066** — 지갑 필드 예약 + (필요시) 외부 로컬-이름 매핑·이름 유일 제약.
2. **중앙 등록·중복 로직** — 이름 네트워크 유일, 충돌=에러(파괴삭제 폐기), 죽은동명 이어받기.
3. **MCP `peer로 등록` + cwd fallback + 미등록 에러.**
4. **라우팅 통일** — notify.rs:567 → 결정적 resolver.
5. **kind 정리** — roster/프론트가 acp/tmux만 산출.
6. **그리드** — 등록전용 표시·인라인 이름·충돌 에러 토스트.
7. **외부 경계** — 로컬 이름 매핑.
각 슬라이스 hermes/codex 실왕복 검증 후 다음.
