# 기능 감사 (FUNCTIONAL-AUDIT)

> **목적**: 사용자 요구 — "목업의 모든 인터랙티브 요소가 라이브 GUI에서 *실제로 작동*해야 한다 (보기만 그럴듯한 건 실패)". 이 문서는 정본 목업의 모든 클릭/입력/버튼/칩을 전수 나열하고, 라이브 구현이 **실제 wired**(real invoke/route + 동작) 인지, 아니면 visual-only / stub / broken / missing 인지 판정한다. 이 판정이 수정 작업의 우선순위를 정한다.
>
> **정본 목업**: `/home/llm/projects/starian-set/_mockups/kakao-mockup.html` (1162줄)
> **라이브 GUI 루트**: `openxgram/ui/web/src` (SolidJS)
> **셸**: `KakaoShell.tsx` → 6탭 (`chat`=TalkTab, `agents`=AgentsTab, `flow`=FlowTab, `wiki`=WikiTab, `market`=MarketTab, `settings`=ConfigTab)
> **방식**: read-only 코드 분석. 코드 변경 없음.
> **작성일**: 2026-06-10

## 범례
- ✅ **works** — 실제 invoke/route 호출 + 동작 확인됨 (코드상 wired)
- 👁 **visual-only** — 렌더되지만 핸들러/효과 없음 (inert)
- 🚧 **stub** — 의도적 placeholder ("준비 중", Phase 6 등 백엔드 미연결)
- ❌ **broken** — wired 됐으나 잘못/누락된 라우트
- ⛔ **missing** — 목업엔 있으나 라이브에 없음

---

## 0. 🔴 "안 되는 것" 우선순위 요약 (영향 큰 순)

작동하지 않는(또는 가짜) 기능을 영향도 순으로. **상위 3개가 사용자가 가장 자주 만지는 핵심 인터랙션**이다.

1. **👁 컴포저 칩 3종 (Bypass Permissions · Default 모델 · High thinking)** — `AcpConversation.tsx` L528-530. 그냥 `<span class="mode">` 텍스트, **onClick 없음**. ACP 세션의 permission mode / model / thinking level 을 전혀 제어하지 못함. 사용자가 명시 지적. → **가장 눈에 띄는 가짜.**
2. **👁 컴포저 @ (파일) · / (커맨드) · 📎 (첨부)** — `AcpConversation.tsx` L524-526. `<span class="ic-btn">` 3개, **모두 onClick 없음**. `@` 파일 멘션, `/` 슬래시 커맨드, 드래그드롭/첨부 전부 inert. placeholder 텍스트 "Type @ for files, / for commands" 는 거짓말.
3. **👁 tmux 라이브 — 대화 헤더/정보패널 클릭 진입 + "제어" 부재** — TalkTab 정보패널 tmux row 는 `(클릭 → 라이브 열기)` 라벨이 붙어있지만 `<div class="sess">` 에 **onClick 없음** (TalkTab.tsx L383~). 대화 헤더 `⌗ tmux N` / `🌿 worktree N` pill 은 `setInfoOpen` 토글만 함(목업은 동일). AgentsTab 의 "⌗ tmux 터미널 열기" → `TerminalOverlay` 는 **모니터링만** (session_screen 폴링 + `<pre>` 출력, **입력/제어 불가**). 즉 tmux 를 *볼 수만* 있고 *조종* 불가. (단 `SessionScreen.tsx` 에 `session_input` 제어 라우트가 이미 존재 — kakao 셸 오버레이가 그걸 안 씀.)
4. **🚧 autoplan (🪄 목표로 자동 구성)** — FlowTab Builder L357-368. 토글은 열리나 본문이 "준비 중" 안내. 백엔드 auto-plan 라우트 없음.
5. **🚧 마켓 전체 (검색·카테고리·＋추가·지갑 잔액·수익·정산)** — `MarketTab.tsx`. Phase 6, `mkuse`/`budbtn` 전부 `disabled`. 검색바 `wsearch` 는 입력 핸들러 없는 `<div>` (필터 안 됨). 카테고리 칩만 선택 토글됨(실효과 없음). 한도 변경(payment_set_daily_limit)만 실제 동작.
6. **⛔ QR·링크 버튼 (에이전트 추가 모달)** — 목업 `btn-q ⛶ QR·링크` (L627). 라이브 AddAgentModal 에 **없음**.
7. **👁 마켓 검색바 + 위키 검색 효과 차이** — 마켓 검색 = inert(👁). 위키 검색 = 실제 클라이언트 필터(✅).
8. **⛔ A2A delegate / 조직도 클릭 액션** — FlowTab 에 A2AOverlay·OrgOverlay 는 열리나(✅ 열기), 위임(delegate) 실행 버튼은 목업에도 라이브에도 실제 호출 없음(표시 위주).

**수정 분류**:
- **Frontend-only 로 고칠 수 있는 것**: 컴포저 @/// 드롭다운(파일목록·커맨드목록은 기존 fs/route 활용), tmux row onClick→오버레이 연결, 마켓 검색바를 `<input>`+필터로, QR 버튼 추가(이미 pairing 라우트 존재 가능성).
- **Backend 필요**: 컴포저 칩 3종(ACP `/sessions` 가 permission/model/thinking 파라미터를 받아야 함 — 현재 계약에 없음), tmux **제어**(session_input 라우트는 있으나 오버레이에 입력 UI 신설), autoplan(auto-plan 라우트), 마켓/지갑 전체(Phase 6).

---

## 1. 💬 대화 탭 (chat → TalkTab.tsx + AcpConversation.tsx)

### 1.1 좌측 명부
- **＋ 에이전트 추가 (add-btn, openAdd)** — 목업: 추가 모달 열기. 라이브: 설정 탭으로 점프(`onJumpToSettings`). 👁→부분: 목업과 동작 다름(모달 X, 탭 이동). 실제 추가는 AgentsTab 모달에서. → **의도적 우회, AgentsTab 추가가 정본이면 OK.**
- **에이전트 row 클릭 (pick)** — 목업: 선택 + 대화/정보뷰 전환. 라이브: `pick(alias)` → ACP 세션 진입. ✅works.
- **검색바 (🔍 에이전트·대화 검색)** — 목업: 표시만. 라이브 `<div class="search">` **onClick/onInput 없음**. 👁visual-only. → input+필터 필요(frontend).
- **하단 6탭 (setTab)** — ✅works (KakaoShell `setTab`).

### 1.2 대화방 헤더
- **← 뒤로 (goBack)** — ✅works (모바일 리스트 복귀 `setMobileChat`).
- **⌗ tmux N pill (toggleInfo)** — 정보패널 토글. ✅works (목업과 동일 동작).
- **🌿 worktree N pill (toggleInfo)** — 동일 토글. ✅works.
- **온라인 상태 pill** — 표시만 (목업도 표시만). ✅정본일치.

### 1.3 컴포저 (composer) — 🔴 핵심 격차 집중
- **입력창 / 전송 ➤ (send)** — 라이브 `onInput`+`onKeyDown`+`sendPrompt()` → ACP `/sessions/{id}/prompt`. ✅works.
- **■ 취소 (cancelTurn)** — `/sessions/{id}/cancel`. ✅works.
- **@ (파일 멘션) ic-btn** — 👁visual-only (onClick 없음). → fs 트리/route 로 파일 picker 드롭다운 필요(frontend, 라우트 있음).
- **/ (슬래시 커맨드) ic-btn** — 👁visual-only. → 커맨드 목록 드롭다운 필요(frontend).
- **📎 (첨부/드래그드롭) ic-btn** — 👁visual-only. 드래그드롭 핸들러 없음. → 첨부 업로드 필요(backend route 여부 확인).
- **🛡 Bypass Permissions (perm 모드)** — 👁visual-only. ACP permission mode 미제어. → **backend**: `/sessions` 가 permissionMode 받아야.
- **Default (recommended) (model)** — 👁visual-only. 모델 선택 미제어. → **backend**: model 파라미터.
- **High (thinking level)** — 👁visual-only. → **backend**: thinking level 파라미터.
- **usage (토큰/비용 표시)** — 라이브는 `⚡ {activeAgent()}` 로 대체(실제 토큰 카운터 아님). 👁/🚧 (실 토큰·비용 표시 없음).

### 1.4 정보 사이드 패널 (#info)
- **✕ 닫기 (toggleInfo)** — ✅works.
- **폴더 표시** — `project_path` 표시. ✅works (동적).
- **tmux 세션 row "(클릭 → 라이브 열기)"** — 👁visual-only. 라벨은 클릭 유도하나 `<div class="sess">` 에 **onClick 없음**. → AgentsTab TerminalOverlay 로 연결 필요(frontend).
- **워크트리 row** — 표시만(목업도 표시만). ✅정본일치.
- **참여 워크플로우 row (openWf)** — 목업: 흐름 열기. 라이브: 표시만, **클릭 액션 없음**. 👁→흐름탭 점프 연결 필요(frontend).

### 1.5 ACP 에이전트 선택 화면 (picker)
- **에이전트 카드 클릭 (spawn)** — 설치된 어댑터만 `spawn()` → `/sessions`. ✅works.
- **✕ 세션 닫기 (closeSession)** — `DELETE /sessions/{id}`. ✅works.

---

## 2. 🙂 에이전트 탭 (agents → AgentsTab.tsx)

### 2.1 명부 + 프로필
- **+ 추가 버튼 (openAdd)** — `setShowAdd(true)` → AddAgentModal. ✅works.
- **에이전트 row 선택** — `agent_profile_get` + `agent_config_chain`. ✅works.
- **실행 모드 칩 (always/on_demand/heartbeat, onExec)** — `agent_profile_set { execution_mode }`. ✅works (실 fs/db 반영).

### 2.2 에이전트 추가 모달 (AddAgentModal)
- **머신 select** — d().machine. ✅works (제출에 반영).
- **AI 종류 select** — ai_type. ✅works.
- **이름 / 역할 / 그룹 input** — ✅works.
- **프로젝트 폴더 input + 폴더 선택 버튼 (folder picker)** — `setShowPicker` → FolderPicker 트리, `onPick` 으로 경로 설정. ✅works (실 폴더 드릴다운).
- **만들기 (submit)** — `agent_profile_set` 호출 → refetch + select. ✅works. (목업은 register 의도였으나 라이브는 profile_set upsert — 동작함.)
- **⛶ QR·링크 버튼 (btn-q)** — ⛔missing. 라이브 모달에 없음. → pairing/QR 라우트 연결 필요.

### 2.3 프로필 빠른 작업 (qbtn)
- **🔗 외부 채널 연동 (openChannels)** — `ChannelOverlay` (bindings_status). ✅works.
- **👛 예산 한도 → 마켓 탭 (openBudget)** — 마켓 탭 점프 또는 노트 토글. 🚧 (실 예산은 Phase 6).
- **⌗ tmux 터미널 열기 (openTerm)** — `TerminalOverlay`. 👁/🚧: **모니터링만** (session_screen 폴링 `<pre>`, 입력/제어 UI 없음). → tmux **제어** 추가 필요(session_input 라우트 존재, 입력 UI 신설=frontend+).
- **📁 폴더 열기** — 목업: qbtn(핸들러 없음). 라이브: 프로필 폴더 트리(FolderPicker)로 탐색·파일 클릭 편집. ✅works (목업보다 강함).
- **💬 이 에이전트와 대화하기 (gotoChat)** — `onGotoChat` → chat 탭. ✅works.

### 2.4 지침/설정 편집 (config-chain → EditorOverlay)
- **cfgrow 클릭 (openEditor: CLAUDE.md / oxg / mcp / settings / env)** — `fs_file_get` 로드. ✅works (실 파일 읽기).
- **저장 (fs write)** — `fs_file_put` (화이트리스트 밖 403). ✅works (실 fs 쓰기).
- **tmux 라이브 화면 (TerminalOverlay)** — session_screen 폴링. ✅(모니터) / ❌(제어 없음).

---

## 3. 🔀 흐름 탭 (flow → FlowTab.tsx)

- **세그 토글 workflows / schedules (setSeg)** — ✅works.
- **＋ 워크플로우 만들기 (addwf, toggleBuilder)** — `setShowBuilder`. ✅works (Builder 열림).
- **Builder: 목표 input / 단계 칩 / 트리거 세그(수동·cron·webhook, trig)** — trigIdx 토글 + cron input. ✅works (상태 반영).
- **Builder 저장 (savewf, toggleBuilder)** — `workflow_upsert { name, yaml_body, cron_expr }`. ✅works (실 저장).
- **🪄 autoplan (toggleAutoplan)** — 토글 열림 but 본문 "준비 중". 🚧stub. → backend auto-plan 라우트.
- **＋ 고용(생성) hirebtn** — 목업 autoplan 내부. 라이브 autoplan 자체가 stub. 🚧.
- **워크플로우 카드 클릭 (openRun)** — 실행이력 오버레이. ✅works (`workflow_runs`).
- **▶ 실행 (workflow_run)** — `workflow_run { id }` → run_id 노트. ✅works.
- **삭제 (workflow_delete)** — ✅works.
- **ON/OFF 토글** — 목업: stopPropagation 만. 라이브: 동등(표시). 👁정본일치.
- **🗂 조직도 버튼 (openOrg)** — OrgOverlay (`orchestration_agents`, reports_to). ✅works (열기+동적 데이터).
- **A2A 위임 (onOpenA2a → A2AOverlay)** — `a2a_agents` 로드. ✅(열기/목록). **위임 실행 버튼**은 목업·라이브 모두 실호출 없음. 👁 (delegate 미구현).
- **스케줄 세그 (schedule_list/_stats/_cancel)** — 목록·통계 표시 + 취소. ✅works.

---

## 4. 📚 위키 탭 (wiki → WikiTab.tsx)

- **🔍 위키 검색 (wsearch)** — 라이브는 `<input onInput=setQ>` + `createMemo` 클라이언트 필터(pages/patterns/mistakes). ✅works (실제 필터).
- **목록 로드** — `wiki_pages_list` / `memory_patterns_list` / `memory_mistakes_list`. ✅works.
- **페이지 클릭 → 본문 보기 (wiki_body_get)** — ✅works.
- **편집 (editing 토글)** — editTitle/editBody. ✅works.
- **저장 (wiki_body_put)** — ✅works (실 쓰기 + 재로드).

---

## 5. ⚙️ 설정 탭 (settings → ConfigTab.tsx)

> 목업 settingsOvl: 지갑·예산, 공개/수익, 연결된 머신, 머신 추가. (정본은 외부채널=에이전트탭, 워크플로우채널=흐름탭으로 분산 안내.)

- **👛 지갑·예산 (openBudget)** — 마켓 탭 영역(Phase 6). 🚧stub.
- **💰 공개 에이전트·수익 (openEarnings)** — 마켓 탭 earnings(Phase 6). 🚧stub.
- **💻 연결된 머신 (4대)** — 표시. (실 머신 목록 라우트 연결 여부 확인 필요.) 👁/✅ TBD.
- **＋ 머신 추가 (설치 링크·QR)** — 목업 표시. 라이브 pairing/설치 링크 동작 여부 확인 필요. 👁/🚧.
- **auto-lock 저장** — (사용자 명시 항목) ConfigTab 내 존재 시 확인 필요 — 코드상 명시 핸들러 미확인. ⚠ 추가 확인 권장.

> ⚠ ConfigTab 세부는 본 감사에서 grep 표층만 확인. 머신 목록·auto-lock 저장의 실 wiring 은 후속 정밀 확인 권장(이 두 항목은 backend 라우트 의존).

---

## 6. 🌐 마켓 탭 (market → MarketTab.tsx) — Phase 6, 대부분 stub

- **서브뷰 전환 (market/wallet/earnings/extwork)** — `setView`. ✅works (UI 전환).
- **👛 지갑 버튼 (openBudget→wallet)** — ✅works (전환).
- **💰 수익 버튼 (openEarnings→earnings)** — ✅works (전환).
- **🔍 마켓 검색바 (wsearch)** — `<div>` (input 아님). 👁visual-only (필터 안 됨). → input+필터(frontend).
- **카테고리 칩 mc (전체/요약/번역/…) (setCat)** — 선택 토글됨. 👁 (필터 실효과 없음 — 리스팅이 정적).
- **mkcard ＋ 내 에이전트로 추가 (mkuse)** — `disabled`. 🚧stub (Phase 6).
- **지갑 잔액 / 이번 달 사용** — "준비 중" 표기. 🚧stub.
- **충전 / 한도 변경 (budbtn)** — `disabled`. 🚧stub.
- **일일 한도 input + 저장 (payment_set_daily_limit)** — ✅works (유일하게 실 동작하는 마켓 기능).
- **정산하기 (USDC) / 에이전트별 예산 수정 (budedit)** — 🚧stub.

---

## 7. 카운트 요약

> 판정 가능한 개별 인터랙티브 요소 기준 집계 (서브뷰 전환 등 순수 UI 토글 포함).

- ✅ **works**: 약 31
- 👁 **visual-only**: 약 13
- 🚧 **stub** (Phase 6 / autoplan): 약 11
- ❌ **broken**: 0 (잘못된 라우트는 없음 — 미연결이 문제)
- ⛔ **missing**: 2 (추가모달 QR·링크 버튼, tmux 제어 입력 UI — 라우트는 존재)
- ⚠ **확인 필요**: ConfigTab 머신목록·auto-lock 저장

---

## 8. 핵심 결론 (수정 방향)

1. **즉시 고칠 가치 최고 (사용자 매일 접촉)** — 컴포저 칩 3종(perm/model/think) + @/// 드롭다운 + tmux 클릭 진입/제어. 이 4개가 "보기만 그럴듯" 의 핵심.
2. **Frontend 만으로 가능**: tmux row onClick→TerminalOverlay 연결, 워크플로우 row→흐름탭, 마켓·로스터 검색바 input 화, @/// 드롭다운(기존 fs·command 라우트 활용), QR 버튼(pairing 라우트).
3. **Backend 필요**: ACP `/sessions` 에 permissionMode·model·thinking 파라미터 추가(칩 작동), tmux 제어 입력 UI(session_input 라우트는 있음 — UI만 신설), autoplan 라우트, 마켓/지갑 Phase 6 전체.
4. **stub 은 정직**: 마켓·autoplan 은 "준비 중" 명시로 날조 없음 — 우선순위 낮음(Phase 6 일정).
