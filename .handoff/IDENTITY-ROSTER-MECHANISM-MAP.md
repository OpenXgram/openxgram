# 신원·로스터·세션 바인딩 — 환경 전체 메커니즘 맵 + 수정 설계

> 마스터 지시: "건드리기 전에 portal·openxgram·AoE가 실제로 어떻게 작동하는지 다 확인해 체계적으로 구성." 3개 시스템 코드 조사(2026-06-23) 결과 종합. 추측 아님, 코드 근거.

## A. 한 에이전트가 생겨서 "메시지 받을 수 있는 상태"가 되기까지 — 실제 흐름

1. **AoE가 세션 생성** (`/home/llm/tools/agent-of-empires/`, Rust):
   - `tmux new-session -d -s aoe_<sanitize(title)>_<uuid앞8자> -c <cwd> <tool>` (`tmux/session.rs:37`, `session/instance.rs:1475`).
   - **AoE는 신원·CLAUDE.md·openxgram 등록을 주입/트리거하지 않음.** cwd + 옵션 `--append-system-prompt`(role/custom_instruction, 실데이터 null)만. openxgram 호출 0건(hooks/mod.rs = 상태파일 셸훅뿐).
2. **Claude Code가 CLAUDE.md 자체 로드**(cwd/`~/.claude`) → 지침에 "시작 시 register_subagent 호출" 있음.
3. **register_subagent**(에이전트가 직접 호출해야, `mcp_serve.rs:1823`): `xgram bot register`로 **별도 봇+keypair(eth) 생성** → agent_capabilities/agent_profiles 작성 + peers 캐시 갱신. **session_identifier는 인자로 줄 때만, 그것도 peers 행이 이미 있을 때만 세팅**(1930). 안 부르면 → 미등록 → 로스터 부재.
4. **auto-seed**(데몬, 시작+30s tick, `daemon.rs:2050`): 로컬 tmux 훑어 `aoe_<alias>_<id>`에서 alias 추출 → `UPDATE peers SET session_identifier='tmux:<세션>' WHERE alias=? AND sid IS NULL`. **SKIP**: `sv_*` prefix·all-numeric·LLM 미검출 pane(2099,2103). 행 생성은 retroactive_register_agents(2359)가.
5. **Portal**: tmux 실시간 폴링(`backend.listWindows()`), 키=**`session:index`**. openxgram alias 연결은 **명명 규칙뿐** — `smart-inject.js`가 peers를 readonly로 읽어 정규식 `^aoe_<alias>_[0-9a-f]+$`로 alias↔세션 매칭. `sv_aoe_<카운터>_<ms>`는 **portal `tmux-backend.js`가 웹뷰용으로 실제 세션 group에 attach한 read-only 그룹 세션**(에이전트 아님).

## B. 그래서 왜 "인박스를 못 본다/엉뚱한 데 간다" — 4개 근본

- **(근본1) 자동 등록 없음**: AoE가 등록을 트리거 안 함 → **각 세션이 스스로 register_subagent 호출해야만** peer가 됨. 안 부르거나 실패(WSL 인증 등)하면 영영 미등록·미바인딩.
- **(근본2) 신원 기준 = 이름 파싱(`aoe_<alias>_<hash>`)**, keypair 앵커 아님: 규칙 안 맞는 이름(`sv_aoe_*`, 또는 자유 세션명)은 alias 해석 실패. 세션 재시작=새 이름이면 바인딩 끊김(sid는 빈 칸일 때만 세팅, 갱신 안 함).
- **(근본3) resolver 두 개가 갈림**: 인바운드 A2A 주입은 `notify::resolve_alias_to_tmux`(notify.rs:567) — **session_identifier 무시 + fuzzy `contains` 부분문자열 매칭** → 엉뚱한 세션 주입. GUI는 `daemon_gui.rs:2876`로 session_identifier 제대로 참조. **둘이 달라서 회신이 엉뚱하게 감.**
- **(근본4) dedup 키 불일치**: reconcile=sid→eth, roster=alias-in-peers. 같은 에이전트가 여러 entry로 쪼개지거나 충돌(`hermes` 두 개 등).

## C. 수정 설계 (마스터 설계 = keypair 앵커 + 자동등록, 이대로)

1. **신뢰 자동등록**: 모든 **실제 working LLM 세션**을 9필드(타이틀·alias·정보주소·머신·종류·세션id·역할·폴더·활성상태)로 로스터에 UPSERT. AoE-spawn 훅 또는 데몬 auto-seed 확장. (`sv_aoe_*` 그룹세션은 실세션의 별칭 뷰이므로 실세션으로 귀속.)
2. **동일성 = 공개키(eth) 앵커**: 로스터 키를 공개키로 통일. 같은 공개키=같은 에이전트→갱신(새 session_id/상태), alias·타이틀은 편집 라벨. 이름 파싱 폐기.
3. **resolver 통일**: 인바운드 A2A(notify.rs:567)도 **session_identifier 결정적 resolver**(daemon_gui.rs:2876과 동일) 사용 → 회신이 대화 발신자 세션으로. fuzzy substring 폐기.
4. **session_identifier 갱신**: 빈 칸일 때만이 아니라 세션 변경 시 권위 갱신.

## D. 검증 (절대 — 중간신호로 "됐다" 금지)
실제 왕복: 보냄→올바른 세션 도달→답신→발신자 화면. **hermes/codex(다른 LLM)가 실동작 확인.** Claude 자가검증·빌드통과·db일치만으론 완료 아님.

## 소스 인덱스
- openxgram: `mcp_serve.rs:1823`(register), `daemon.rs:2050`(auto-seed)/2099(sv_skip)/1358-1660(inbound), `notify.rs:567`(fuzzy resolver), `daemon_gui.rs:2876`(결정적 resolver), `identity.rs:173/291/398`(reconcile/roster), `bot.rs`(keypair).
- portal: `lib/webhook.js`(매칭4단계+주입), `lib/smart-inject.js`(alias↔세션 정규식), `lib/aoe-backend.js`, `/home/llm/.starian/portal/terminal/tmux-backend.js:414`(sv_aoe 생성).
- AoE: `/home/llm/tools/agent-of-empires/` `tmux/session.rs:37`(이름), `agents.rs:142`, `session/instance.rs:1475`. openxgram 연동 0.
