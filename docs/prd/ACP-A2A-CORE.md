# ACP + A2A 통신 코어 — 정본 아키텍처

> 2026-06-10 마스터 확정. OpenXgram GUI 재설계의 **본질**. 옛 "peer=터미널/tmux" 모델은 폐기.
> 이 문서가 통신·에이전트 모델의 정본. 충돌 시 이 문서 우선.

## 1. 핵심 원칙

- **에이전트 = 마스터가 생성한 것** (`agent_profiles`: alias, ai_type, classification, machine, project_path, role, group, execution_mode, worktree, is_public). 로스터 = 이것만.
- **tmux ≠ 에이전트.** tmux는 에이전트가 *돌아가는 곳*일 뿐 → 에이전트 **상세**에만 표시.
- **통신 = ACP + A2A (메인).** tmux send-keys 주입 = 폐기.

## 2. 통신 모델

### 2.1 나(GUI/마스터) ↔ 에이전트 = **ACP** (Agent Client Protocol, Zed)
- 에이전트와의 대화 = **ACP 세션**. 데몬이 에이전트의 `ai_type` → ACP 어댑터
  (claude→claude-agent-acp / codex→codex-acp / gemini→gemini --acp)로 spawn, `cwd = project_path`.
- 프롬프트 → `session/prompt` → `session/update` 실시간 스트리밍.
- **이미 작동**: `openxgram-acp` 크레이트(B-1~B-3) + `/v1/acp/*` 라우트 + 실시간 스트리밍 e2e 증명(rc.296).
- ⚠️ 정정: "ACP 세션"은 별도 기능이 아니라 = **에이전트와 대화하는 유일한 방식**. 별도 진입점 제거, 통합.

### 2.2 에이전트 ↔ 에이전트 = **A2A** (Agent2Agent, Google)
- 한 에이전트가 다른 에이전트에게 작업 위임/호출 = A2A (AgentCard 발견 `/.well-known/agent-card.json` + `tasks/send|get|cancel`).
- cross-machine 에이전트 협업도 A2A.
- `openxgram-a2a` 크레이트 존재하나 **미배선** → 데몬 라우트 + GUI 배선 필요.

### 2.3 tmux = 런타임 표시만
- 에이전트가 tmux에서 돌면 상세 패널 "실행 중 tmux"에 표시(`daemon_gui_sessions.rs` 로컬 탐지). **통신 아님.**

## 3. 코드 매핑 / 폐기

- 에이전트 모델: `agent_profiles`(생성). 로스터 = `agent_capabilities WHERE role != 'tmux'` (rc.301). 
  → 추후 `agent_profiles` 정본화 (add 흐름이 profiles에도 쓰도록).
- ACP: `openxgram-acp` + `/v1/acp/*` (작동) → **대화 기본 경로로 승격**.
- A2A: `openxgram-a2a` → mcp_serve/데몬 라우트 + GUI 배선 (신규).
- **폐기**: `auto_seed_local_tmux_agents`(에이전트 등록 부분, rc.301 제거됨), `peer_send` tmux-inject(메인 통신에서).

## 4. 단계

1. ✅ **에이전트 모델 정정** — 로스터=생성 에이전트, tmux=상세 (rc.301).
2. **ACP 대화 승격** — TalkTab 대화 = 선택 에이전트를 `ai_type` 어댑터로 ACP 구동. 별도 "ACP 세션" 통합. `execution_mode`로 상시/깨움.
3. **A2A 배선** — 에이전트↔에이전트 (`openxgram-a2a` → 데몬 라우트 + GUI "조직도/위임" UI).
4. **tmux-inject 통신 폐기** — peer 대화를 ACP로 일원화 후 tmux send-keys 경로 제거(또는 레거시 격리).

## 5. 살아있는 자산 (재사용)
- 카톡 셸 UI·목업 화면·기능 배선(위키/파일/머신/첨부)·PWA·모바일·디스크 서빙(빌드 즉시화) — 전부 유효.
- `openxgram-acp` 크레이트 = ACP 코어 (작동 증명).
