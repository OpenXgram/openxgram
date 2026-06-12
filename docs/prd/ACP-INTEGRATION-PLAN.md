# ACP Integration — Design & Decision Plan

> 작성: 2026-06-10 20:36 KST · 상태: DESIGN/PLAN (코딩 전 마스터 승인용)
> 범위: GUI 대화(conversation)를 위한 실질 ACP 통합. 읽기 전용 조사 기반.
> 정본 참조: `_mockups/PHASE3-PLAN.md`, `_mockups/GUI-MIGRATION-PLAN.md` §3,
> `docs/research/acp-core-integration.md`, `CONCEPT.md`(starian-set 루트).

---

## 1. 이 프로젝트에서 "ACP"란 무엇인가 (용어 정리)

이 작업의 "ACP"는 **Agent Client Protocol (Zed, agentclientprotocol.com)** —
JSON-RPC 2.0 over stdio, 클라이언트가 에이전트를 서브프로세스로 띄워
`session/new` → `session/prompt` → `session/update` 스트림으로 구동하는 **대화 세션 프로토콜**이다
(`docs/research/acp-core-integration.md` §1).

명확히 구분 — 같은 약자 3종이 섞여 있다:
- **Agent Client Protocol** (Zed) = 이 대화 작업의 대상. ✅
- **IBM ACP** (BeeAI) = `CONCEPT.md:350` "미채택 (A2A로 대체)". ❌ 이 작업 아님.
- **Virtuals ACP** = Agent Commerce Protocol, 블록체인 상거래 (`CONCEPT.md:349,656`). ❌ 통신 아닌 상거래.

마스터가 "real ACP"를 말할 때 의미하는 것:
현재 대화는 두 가지 비-ACP 경로로 돌아간다 — (1) peer 간은 `peer_send`(아래 §3),
(2) 로컬 세션 화면은 tmux `capture-pane` 스크린 스크랩(`gui_session_screen`).
"real ACP"는 이 tmux 스크린-스크랩 자리에 **표준 세션 프로토콜**
(session.create / prompt / stream + 읽음표시)을 두는 것이다. peer_send 대체가 아니라 **세션 본문 레이어** 문제다.

---

## 2. 정직한 현재 상태

- **대화 = ACP 아님.**
  - peer↔peer 메시지: `peer_send.rs` — HTTP/Nostr `Envelope`(openxgram-transport)로 직접 송신.
    outbox/inbox SQLite 저장 + outbound_queue ACK 추적(rc.219/227). ACP 세션 개념 없음.
  - 로컬 세션 본문: `daemon_gui.rs`의 `gui_session_screen`(tmux capture-pane 미러) +
    `gui_session_input`(tmux send-keys). 즉 **화면 스크랩**이지 구조화 프로토콜 아님.
  - 통합 대화 뷰: `gui_peer_conversation`(daemon_gui.rs:7118) — alias variant OR 매칭으로
    outbox/inbox 세션을 timestamp ASC 통합. 읽기 전용 집계.

- **a2a / anp 크레이트 = 존재하나 ACP 아님, 그리고 unwired.**
  - `openxgram-a2a` = **Google/A2A (a2a-protocol.org) Agent2Agent** 클라이언트.
    AgentCard 발견(`/.well-known/agent-card.json`) + `tasks/send|get|cancel` JSON-RPC. **client-only, 영구데이터 없음**
    (`crates/openxgram-a2a/src/lib.rs` doc-comment).
  - `openxgram-anp` = **ANP (Agent Network Protocol)**. did:wba 분산 발견 + DID 서명 HTTP.
    `anp_announce_self`는 명시적 **stub**(`src/mcp.rs:6`).
  - **둘 다 ACP가 아니다** — A2A·ANP는 *외부 에이전트 호출/발견*(outbound interop)용. 세션 호스팅 없음.
  - 워크스페이스 멤버엔 등재돼 있으나(`Cargo.toml:27-28`), `openxgram-cli`(mcp_serve.rs/Cargo.toml)에서
    **참조 0** — 즉 빌드만 되고 **런타임 미연결**. 크레이트 내 stale 주석("NOT yet listed in members")은 부정확.
  - PHASE3-PLAN 3-0 결론: a2a/anp는 "Phase 3-A 세션 추상화엔 **부적합**, 그대로 둔다(미래 agent-to-agent 호출용, orthogonal)".

- **이미 깔린 토대(재사용 가능):**
  - 읽음표시: migration 0040 `message_ack`(ack_status: sent→delivered/read/processing/done/failed) **라이브**.
  - 세션 식별자 풀: `gui_sessions`(daemon_gui.rs:1213, `tmux:`/`peer:`/`aoe:`/`claude:` prefix).
  - 기록 영속: SQLite messages/sessions — 재부팅 복원 토대 존재.
  - 실행모드 생명주기: Phase 2에서 `agent_profiles.execution_mode` 구현·배포(rc.289) — 호스팅 게이트 사실상 해소됨(§5).

---

## 3. peer 경로 — ACP가 무엇을 바꾸나

- **바뀌지 않는 것**: peer↔peer 송신(`peer_send` HTTP/Nostr Envelope)은 cross-machine 신원·서명·ACK의 본질.
  ACP는 이걸 대체하지 않는다(대체하면 신원/서명 깨짐, oxg.md §6 #5).
- **바뀌는 것**: GUI 대화방이 **로컬 에이전트 세션을 구동/스트리밍하는 방식**.
  현재 tmux capture-pane 스크린 스크랩 → 구조화 세션 프로토콜(create/prompt/stream/ack)로 교체.
- 따라서 ACP 도입은 **augment**(세션 레이어 추가)이지 peer_send **replace**가 아님.

---

## 4. 옵션 (2~4)

각 옵션: 무엇을 만드나 / 노력(S·M·L) / 장단 / 사용자에게 주는 것.

- **옵션 A — 기존 tmux 위 얇은 네이티브 세션 프로토콜 (PHASE3-PLAN 권장안)**
  - 만드는 것: NEW `daemon_acp.rs`(`/v1/acp/sessions/create|prompt|stream`), `daemon_acp_messages.rs`(`/ack`).
    create=식별자 풀+execution_mode 해석, prompt=tmux send-keys 재사용, stream=capture-pane → SSE/WS, ack=message_ack 노출. (~500 LOC)
  - 노력: **M**.
  - 장점: 검증된 tmux 경로 재사용·로컬 멀티-AI(claude/codex/gemini) 즉시 지원·재부팅 복원 토대 있음·신원경로 무손상.
  - 단점: Zed ACP **스펙 와이어 호환은 아님**(자체 REST). 외부 ACP 클라이언트와 직접 상호운용 불가.
  - 사용자 효과: 카톡식 대화방에 읽음표시·스트리밍·툴콜 렌더가 바로 보임. **GUI 검증 가능.**

- **옵션 B — Zed ACP 스펙 준수 `openxgram-acp` 크레이트 (research 문서안)**
  - 만드는 것: NEW `crates/openxgram-acp`(client.rs/session.rs/mcp.rs), 서브프로세스 spawn +
    JSON-RPC stdio + `session/update` 릴레이 + `fs/*`·permission 콜백. mcp_serve 디스패치 + 데몬 프로세스 레지스트리.
    실제 `claude-agent-acp`(npm) 연동.
  - 노력: **L**.
  - 장점: 진짜 스펙 호환 — `codex-acp`/`gemini --acp`/`opencode acp` 등 임의 ACP 에이전트 plug-in.
    runtime-in-runtime·stdio 오염·full-duplex 데드락 회피 설계가 문서에 이미 정리됨.
  - 단점: 비용·리스크 최대(서브프로세스 lifecycle/zombie reaper, npm 의존, 스펙 추적). 기존 tmux 자산 대부분 우회.
  - 사용자 효과: 외부 표준 에이전트를 GUI에서 직접 구동. 단 가치 실현까지 시간 김.

- **옵션 C — A·B 하이브리드 (네이티브 레이어 + ACP 어댑터 어드밴티지)**
  - 만드는 것: 옵션 A를 먼저 출하 → `/v1/acp/*`를 **추상 인터페이스**로 두고, 백엔드 드라이버를
    `tmux`(A) 또는 `acp-subprocess`(B 크레이트)로 교체 가능하게. B는 후속 단계로 분리.
  - 노력: A=M 선출하, B=L 후속.
  - 장점: 빠른 GUI 가치 + 미래 스펙 호환 경로 보존. 게이트별 결정 분리.
  - 단점: 인터페이스 설계 신중 필요(드라이버 추상화 비용).
  - 사용자 효과: 지금 대화 동작 + 나중에 외부 ACP 에이전트 확장.

- **옵션 D — defer (지금 안 함)**
  - 만드는 것: 없음. 현 tmux 스크린-스크랩 유지.
  - 노력: **S(0)**.
  - 단점: 대화방이 스크린 미러에 머물러 읽음표시·스트리밍·툴콜 렌더 미흡. Phase 3 미완.

---

## 5. 마스터가 내려야 할 결정

- **결정 1 — 호스팅 모델 (게이트 항목, GUI-MIGRATION-PLAN:101).**
  PHASE3-PLAN은 **B안(혼합) 이미 채택**으로 기록 — Phase 2 `agent_profiles.execution_mode`로 구현·배포됨:
  `always`(프라이머리=상시 tmux) / `on_demand`(프로젝트=메시지 도착시 부팅, idle 유지|종료) / `heartbeat`(특수=큐 호출시 부팅→실행→잠듦, 큐는 Phase 4).
  - 항상-켜둠: 무엇이 도나=tmux 세션 상시. 비용=메모리/세션 점유 지속. 재부팅=재기동 필요(복원 토대 있음). 지연=0(즉답).
  - 깨움(on_demand): 무엇이 도나=평소 미기동, 메시지시 exec+tmux ready 대기(~5s). 비용=idle시 0. 재부팅=다음 메시지에 자동 부팅. 지연=첫 메시지 ~5s.
  - **확인 요청**: 이 B안 채택을 그대로 승인하는가? (이미 배포된 상태이므로 사실상 재확인.)

- **결정 2 — 프로토콜/크레이트.** 옵션 A(네이티브) / B(Zed 크레이트) / C(하이브리드) 중 택1.
  a2a·anp는 ACP가 아니므로 **세션 레이어 후보에서 제외** 확정(미래 outbound interop용으로 보존).

- **결정 3 — 범위.** GUI 대화 세션 한정인가, 아니면 모든 peer 통신까지인가?
  권장: **GUI 대화 세션 한정**. peer_send(신원·서명·cross-machine)는 ACP로 대체하지 않음.

---

## 6. 권장 경로 + 단계 + 리스크

**권장 = 옵션 C (실질적으로 A를 먼저 출하, B 인터페이스 보존).**
이유: 검증된 tmux 자산 + message_ack + execution_mode가 이미 라이브 → 빠른 GUI 가치(원칙 #5 UI 검증),
동시에 미래 Zed-스펙 호환 문(드라이버 교체) 남김. a2a/anp는 손대지 않음.

단계(의존성 순, PHASE3-PLAN 3-A~3-C 정렬):
- **3-A 백엔드 ACP 레이어**: `daemon_acp.rs`(create/prompt/stream) + `daemon_acp_messages.rs`(ack).
  호스팅 분기는 `agent_profiles.execution_mode`에 위임(결정 1). 착수 전 token-savior
  `get_change_impact(daemon.rs/daemon_gui.rs)` + `find_semantic_duplicates(session 핸들러)` (훅 강제).
  ⚠️ **게이트 결정 1·2를 여기서 콜아웃** — 드라이버 추상화 경계를 A 단계에 박는다.
- **3-B 프론트 대화 본문**: KakaoShell 대화 탭 — 내 말풍선/에이전트 전체폭(툴콜·코드블록) + 컴포저(권한·모델·사고수준·토큰미터). 읽음표시 노출.
- **3-C 재부팅 복원 + 깨움 부팅**: 재부팅 후 대화방·기록 복원 + on_demand 에이전트 메시지시 자동 부팅 검증.

리스크:
- 와이어-스펙 미호환(옵션 A 한계) — C의 드라이버 추상화로 후속 B 흡수 가능하게 설계해 완화.
- on_demand 부팅 지연(~5s) UX — 컴포저에 "부팅 중" 상태 노출.
- tmux 스크린 스크랩 → 구조화 전환 시 기존 ChatTab/SessionScreen 회귀 — 점진 교체(엔드포인트 병행).
- 절대규칙: peer_send/신원 경로 손대지 말 것, silent fallback 금지(thiserror 명시 variant), DB 마이그레이션 시 기존 데이터 보존.
