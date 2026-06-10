# 목업 충실도 감사 (MOCKUP-FIDELITY-AUDIT)

> **목적**: 사용자가 반복적으로 "라이브 GUI가 목업이랑 너무 다르다(목업이랑 너무 달라)"고 지적함. 기존 수정이 piecemeal 이라 계속 누락됨. 이 문서는 **정본 목업 ↔ 라이브 구현**을 화면 단위로 전수 비교하여 모든 divergence 를 한 번에 잡을 수 있게 한다.
>
> **정본 목업**: `_mockups/kakao-mockup.html` (단일 파일, 1162줄)
> **라이브 GUI 루트**: `openxgram/ui/web/src` (SolidJS)
> **셸**: `components/KakaoShell.tsx` — 하단 6탭 → 탭별 컴포넌트
> **감사 방식**: read-only. 코드 변경 없음. 이 문서가 수정 작업의 기준선.
> **작성일**: 2026-06-10

---

## 0. 핵심 요약 — 가장 눈에 띄는 격차 (우선순위 순)

사용자가 화면을 클릭하며 돌아다닐 때 **"너무 다르다"고 느끼는 주범 TOP 5**:

1. **🌐 마켓 탭이 통째로 빈 Placeholder** — 목업 `marketOvl`(L818-854)은 지갑 배지 + 카테고리 칩 + 2열 에이전트 카드 그리드(유튜브 요약봇/PDF 번역기/이미지 생성봇/코드리뷰봇)로 가득 찬 화면. 라이브는 `KakaoShell.tsx` L23 `Placeholder` 컴포넌트("Phase 6에서 구현됩니다")만 표시. **하단 탭 6개 중 1개가 사실상 빈 화면** — 클릭 즉시 가장 크게 체감.

2. **👛 지갑·예산 / 💰 수익 / 🔗 외부 채널 화면 전부 미구현** — 목업의 `budgetOvl`(L856-905), `earningsOvl`(L936-965), `channelOvl`(L967-1002), `extWorkOvl`(L924-934)은 완성된 보드 화면. 라이브엔 해당 컴포넌트도, CSS 클래스(`.budrow .earn-item .chcard .budtop`)도 **0개**. 에이전트 프로필의 진입 버튼은 `disabled` 처리된 죽은 버튼(`AgentsTab.tsx` L196 "👛 예산 한도 · 외부 채널 연동 · 공개/수익 (Phase 6)").

3. **🔀 흐름 탭이 목업의 절반만 구현** — 목업 `wfOvl`(L634-731)은 ① 빌더(프로젝트 선택 + 목표 입력 + 🪄 자동구성/autoplan + 단계 칩 드래그 + 채널 연결 select) ② **프로젝트별 그룹핑**(`📁 starian-set … 🗂 조직도` 헤더) ③ 워크플로 카드(trigger 칩·on/off·steps 파이프라인·runline)로 구성. 라이브 `FlowTab.tsx`는 빌더 없음, autoplan 없음, 프로젝트 그룹핑 없음(주석 L7 "project 필드 없어 flat 렌더"), **조직도/실행이력 진입 없음**. 카드 + 스케줄 세그먼트만 존재.

4. **🗂 조직도(orgOvl) · 실행이력(runOvl) · tmux 터미널 미리보기(termOvl) 전부 미구현** — 목업의 3개 오버레이(org L802-816, run L759-799, term L1004-1024)는 각각 클릭 진입점(흐름 카드/세션 칩)에서 열리는 핵심 인터랙션. 라이브엔 컴포넌트·CSS·진입점 모두 없음. 특히 터미널 미리보기는 "라이브 tmux"를 보여주는 데모의 하이라이트인데 완전 부재.

5. **에이전트 프로필 사이드 섹션이 목업과 다른 곳에 흩어짐** — 목업 친구탭 프로필(`agentView` L527-568)은 ① 정보 그리드 ② 실행모드 ③ **예산 한도 / 외부 채널 / 공개·수익 빠른버튼**(L556-560, 실제 onclick 으로 budget/channel/earnings 오버레이 열림) ④ tmux 터미널 열기 빠른작업. 라이브 `AgentsTab.tsx`는 ③을 단일 `disabled` 버튼으로 뭉개고, tmux/워크트리/워크플로 섹션은 아예 **TalkTab 정보패널로 이동**시켜 프로필에서 사라짐.

> 한 줄 결론: **하단 탭 6개 중 "마켓"은 빈 화면, "흐름"은 반쪽**, 그리고 목업에서 클릭으로 열리는 **6개 보조 오버레이(지갑·수익·채널·조직도·실행이력·터미널) 전부가 미구현**. 이게 "너무 다르다"의 실체.

---

## 1. 목업 화면 전수 목록 (정본 인벤토리)

`kakao-mockup.html`이 담은 모든 화면/뷰 (라인 범위 포함):

**메인 셸 (좌측 명부 + 우측 본문)**
- 좌측 명부 + 하단 6탭 (L381-457): `side-top`/`search`/그룹 group-title/row + tabs(에이전트·대화·흐름·위키·마켓·설정)
- 우측 대화방 chatView (L459-525): `chat-top`(온라인/tmux/worktree pill) + `msgs`(내 말풍선 `.me` + 에이전트 전체폭 `.agent`) + Claude-Code 다크 컴포저(`bar-l` 권한/모델/think + `bar-r` usage/send)
- 친구탭 에이전트 정보 뷰 `agentView` (L527-568): 정보 그리드 + 실행모드 + 예산/채널/수익 버튼 + 빠른작업
- 우측 상세 패널 `#info` (L572-595): 폴더 / 실행중 tmux / 워크트리 / 참여중 워크플로우

**오버레이/모달 (클릭 진입)**
- 에이전트 추가 8단계 모달 `#ovl .modal` (L598-632): 머신·폴더·AI종류·이름·역할·분류·그룹·실행모드·워크트리·공개 + QR/만들기
- 워크플로우 보드 `#wfOvl` (L634-731): 빌더 + autoplan + 프로젝트 그룹 + 워크플로 카드 + runline
- LLM 위키 보드 `#wikiOvl` (L733-757): 검색 + 기록·기억 + 🧬 자기개선(패턴/실수→규칙)
- 실행이력 상세 `#runOvl` (L759-799): run 행 토글 + 단계별 ✓/skip/error + runlog
- 프로젝트 조직도 `#orgOvl` (L802-816): orgnode 트리 + 리드/특수 badge
- OpenAgentX 마켓 `#marketOvl` (L818-854): 지갑 배지 + 검색 + 카테고리 칩 + 2열 mkcard 그리드
- 지갑·예산 `#budgetOvl` (L856-905): budtop(잔액/사용/충전) + 에이전트별 예산 budrow + 충전/사용 내역 txn
- 설정 `#settingsOvl` (L907-921): 계정·지갑 qbtn 행
- 외부 작업 상세 `#extWorkOvl` (L924-934): 요청 → 에이전트 작업 결과
- 공개 에이전트 수익 `#earningsOvl` (L936-965): budtop(수익) + 공개 에이전트 earn-item + 외부 사용 이력
- 외부 채널 바인딩 `#channelOvl` (L967-1002): 데몬 안내 + chcard(Discord/Slack) + 수신 파일 처리
- 라이브 tmux 터미널 미리보기 `#termOvl` (L1004-1024): 다크 터미널 + 라이브 로그
- 지침/MCP 에디터 `#edOvl` (L1027+)

**총 메인 영역 4 + 오버레이/모달 14 = 18개 화면.**

---

## 2. 화면별 비교 (목업 ↔ 라이브)

### 2.1 좌측 명부 + 하단 6탭
- **목업**: L381-457 · **라이브**: `KakaoShell.tsx`(탭 셸) + 각 탭 좌측 roster (`TalkTab` `kk-talk-roster`, `AgentsTab` `kk-roster`)
- **상태**: ⚠️ 부분
- **격차**:
  - 목업은 좌측 명부가 **셸 공유 1개**(모든 탭이 같은 명부 공유). 라이브는 탭마다 별도 roster(`TalkTab` 좌측, `AgentsTab` 좌측)로 분리 → 구조 상이.
  - 목업 `add-btn` "＋ 에이전트 추가"(L385) → 라이브 `AgentsTab`은 `kk-add` "+ 추가"(L43), `TalkTab`은 "⚡ ACP 세션" + "＋ 에이전트 추가"(L381-382). 버튼 라벨/개수 다름.
  - 그룹 헤더: 목업 `group-title` + `gt-sub`("⭐ 통합관리 · 프라이머리 (전체 1 · ⚡상시)") L391. 라이브 `kk-gt`는 `(개수)`만 — gt-sub 의 실행모드 요약 텍스트 누락.

### 2.2 우측 대화방 (chatView)
- **목업**: L459-525 · **라이브**: `TalkTab.tsx` L455-561
- **상태**: ✅ 대체로 일치 (라이브가 충실 이식)
- **격차(소)**:
  - 읽음 표시: 목업 `.me .rr.read "읽음"`(L477) — 라이브 컴포저/메시지에 읽음 배지 렌더 확인 필요(`.read` CSS 존재하나 메시지 렌더에서 적용 여부 미확정).
  - usage: 목업 "60k/1.00M (6%) · $0.6923"(L519) — 라이브 "0 / 1.00M (0%)"(L549), 비용($) 부분 없음. (실데이터라 허용 범위, 단 $ 표기 형식 누락.)
  - chat-top pill: 목업 3개(온라인/tmux2/worktree1) ↔ 라이브 동일 3개 — 일치.

### 2.3 에이전트 정보 뷰 (agentView / 프로필)
- **목업**: L527-568 · **라이브**: `AgentsTab.tsx` L82-210 (`kk-prof`)
- **상태**: ⚠️ 부분 (가장 중요한 격차)
- **격차**:
  - 정보 그리드: 목업 6카드(AI종류/분류/머신/폴더/역할/+) ↔ 라이브 8카드(+그룹/공개/워크트리). 라이브가 더 풍부 — OK.
  - **예산/채널/수익 버튼 죽음**: 목업 L556-560은 3개의 동작하는 qbtn(`openBudget()`/`openChannels()`/`openEarnings()`). 라이브 L196은 **단일 `disabled` 버튼** "👛 예산 한도 · 외부 채널 연동 · 공개/수익 (Phase 6)". → 진입점 자체가 죽어 있음.
  - **tmux 터미널 열기 빠른작업 없음**: 목업 L564 "⌗ tmux 터미널 열기"(openTerm) + L565 "📁 폴더 열기". 라이브 빠른작업은 "💬 이 에이전트와 대화하기" 1개뿐(L201).
  - 동적 설정탐지(config-chain)는 라이브가 추가 구현(L149-188) — 목업엔 없는 plus. OK.

### 2.4 우측 상세 패널 (#info)
- **목업**: L572-595 (친구탭 프로필 옆) · **라이브**: `TalkTab.tsx` L562-595 (대화탭 info 패널)
- **상태**: ⚠️ 부분 (위치 이동)
- **격차**:
  - 목업은 이 패널이 **에이전트(친구)탭** 맥락. 라이브는 **대화탭**으로 이동 — 사용자가 목업 기대 위치(프로필)에서 못 찾음.
  - 섹션 자체(폴더/tmux/워크트리/워크플로우)는 라이브 TalkTab 에 이식됨 — 내용은 보존.
  - 목업 tmux 세션 클릭 → `openTerm()` 터미널 열림(L581). 라이브엔 터미널 오버레이 없으므로 **세션 클릭이 죽음**.
  - 목업 워크플로우 클릭 → `openWf()`(L590). 라이브 흐름 연결 여부 미확정.

### 2.5 에이전트 추가 8단계 모달
- **목업**: L598-632 (`.modal` 2열 그리드 폼) · **라이브**: `AgentsTab.tsx` L230+ (`kk-modal`, step 기반 위저드)
- **상태**: ⚠️ 부분 (UX 패턴 상이)
- **격차**:
  - 목업은 **한 화면에 전 필드 노출**(2열 mrow 그리드, 번호 1~8 라벨). 라이브는 **step-by-step 위저드**(`cur().title`, `setStep`) — 한 번에 한 필드. 구조·체감 크게 다름.
  - 목업 footer: "⛶ QR·링크" + "만들기"(L627-628). 라이브에 QR·링크 버튼 확인 필요(미발견 가능성).
  - 목업 실행모드 seg "⚡ 상시 / 😴 깨움"(L621), 워크트리 seg, 공개 seg 모두 한 화면. 라이브는 분산.

### 2.6 워크플로우 보드 (wfOvl) → 흐름 탭
- **목업**: L634-731 · **라이브**: `FlowTab.tsx` (`kk-flow`)
- **상태**: ⚠️ 부분 (절반)
- **격차**:
  - **빌더 전체 없음**: 목업 L641-686 빌더(프로젝트 select + 🎯 목표 input + 🪄 autoplan 자동구성 + 단계 칩 stepchips 드래그 + 채널 연결 select + /run·/status 안내). 라이브 FlowTab 엔 만들기 빌더 UI 부재.
  - **autoplan(목표 자동구성) 없음**: 목업 L650-686 핵심 데모 기능. 라이브 0.
  - **프로젝트 그룹핑 없음**: 목업 `wfproj` 헤더("📁 starian-set … 🗂 조직도" L716). 라이브 주석 L7 "project 필드 없어 flat 렌더" → 그룹 없이 평면.
  - **🗂 조직도 진입 없음**: 목업 `orgbtn onclick=openOrg()`. 라이브 없음.
  - **실행이력(runline 클릭→runOvl) 없음**: 목업 카드 클릭 `openRun()`. 라이브 runline 표시는 있으나 상세 진입 없음.
  - 일치: 워크플로 카드 자체(`wfcard/wftop/trig/onoff/goal/runline/rdot`)는 CSS 이식됨(kakao.css 주석 L407). 트리거 칩·on/off·steps 렌더 존재.
  - 라이브 추가: 스케줄(cron) 세그먼트(`schedules`) — 목업엔 별도 화면 아니나 흐름 내 합리적 확장.

### 2.7 LLM 위키 보드 (wikiOvl) → 위키 탭
- **목업**: L733-757 · **라이브**: `WikiTab.tsx` (`kk-wiki`)
- **상태**: ✅ 일치 (verbatim 이식)
- **격차(소)**:
  - 라이브가 정본 `.wsearch/.wsec/.witem/.wt2/.wm/.wkind` 마크업·CSS 그대로 포팅(컴포넌트 주석 L4-6). 샘플 텍스트만 실데이터로 치환.
  - 의미검색 명령 없어 `.wsearch`는 클라이언트 필터(주석 L11) — 동작 약함이나 구조 일치.
  - 칩 매핑: 목업 `wk-insight/wk-decision/wk-evo/wk-order` ↔ 라이브 `KIND_CHIP`(L36-) 매핑 — 일치.

### 2.8 실행이력 상세 (runOvl)
- **목업**: L759-799 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: 컴포넌트·CSS(`.runrow .rh .rt .rg .rstat .runsteps .rstep .si .sd .runlog`)·진입점 전부 없음. 목업의 run 토글/단계별 ✓·skip·ERROR·runlog 표시 부재.

### 2.9 프로젝트 조직도 (orgOvl)
- **목업**: L802-816 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: `.orgnode .oava .on2 .or .obadge(ob-lead/ob-special) .orgindent` CSS 없음. 트리·리드/특수 badge·진입(openOrg) 전부 부재.

### 2.10 OpenAgentX 마켓 (marketOvl) → 마켓 탭
- **목업**: L818-854 · **라이브**: `KakaoShell.tsx` L23 `Placeholder` ("Phase 6에서 구현")
- **상태**: ❌ 미구현 (빈 Placeholder)
- **격차**: 지갑 배지(`.wallet` "👛 지갑 $42.50 · 한도 $100"), 검색(`.wsearch`), 카테고리 칩(`.mkcat .mc` 전체/요약/번역/이미지/리서치/코딩/SNS), 2열 카드 그리드(`.mkgrid .mkcard .mkh .ava .mkn .mkby .mkprice .mkd .mkuse` — 유튜브요약봇/PDF번역기/이미지생성봇/코드리뷰봇), "💰 내 공개 에이전트·수익" 버튼 — **전부 없음**. CSS 클래스 `.mkcard/.mkcat/.mkgrid/.mkprice/.mkuse` kakao.css 에 0개.

### 2.11 지갑·예산 (budgetOvl)
- **목업**: L856-905 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: `.budtop .big .cap .budbtn`(잔액/이번달 사용/충전·한도변경), `.wsec`+에이전트별 예산 `.budrow .bn .pbar .budamt .budedit`, 충전/사용 내역 `.txn .ti .tn .ts .ta(plus/minus)` — 전부 부재. 설정탭/마켓/프로필에서 `openBudget()` 진입점도 죽음.

### 2.12 설정 (settingsOvl) → 설정 탭
- **목업**: L907-921 · **라이브**: `ConfigTab.tsx` (`kk-set`)
- **상태**: ⚠️ 부분 (방향 다름)
- **격차**:
  - 목업 설정은 **sparse**: 계정·지갑 qbtn 행("👛 지갑·예산", "💰 공개 에이전트·수익") 위주 — 즉 **다른 오버레이로 가는 런처**.
  - 라이브 ConfigTab 은 **실데이터 설정 화면**으로 재해석: 계정·신원 그리드, 자동잠금, 연결된 머신, 데몬 연결 URL/토큰 — 목업의 지갑/수익 런처 qbtn **없음**(해당 오버레이 미구현이라).
  - 버전 표기 `.ver`(L133) — 라이브 추가, OK.
  - 결론: 라이브가 더 실용적이나 목업의 "지갑·수익 진입" 의도는 빠짐.

### 2.13 외부 작업 상세 (extWorkOvl)
- **목업**: L924-934 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: 요청 텍스트 + "에이전트 작업 결과"(`.wsec` + `.agent .body`) 부재. 수익 이력 클릭 진입(openExtWork) 없음.

### 2.14 공개 에이전트 수익 (earningsOvl)
- **목업**: L936-965 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: `.budtop`(이번달/누적 수익 + 정산하기 USDC), 공개 에이전트 `.earn-item .earn-top .en .erate .er .earn-stat`, 최근 외부 사용 이력 `.txn` — 전부 부재.

### 2.15 외부 채널 바인딩 (channelOvl)
- **목업**: L967-1002 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: 데몬 직접처리 안내 박스, "＋ 채널 연결" 버튼, `.chcard .chtop .chav(ch-dc/ch-sl) .chn .chs .chbind .bt .chdir`(Discord/Slack 바인딩), 📎 수신 파일 처리 `.filebox .fileitem .fi .fm` — 전부 부재. 프로필 `openChannels()` 진입점 죽음.

### 2.16 라이브 tmux 터미널 미리보기 (termOvl)
- **목업**: L1004-1024 · **라이브**: MISSING
- **상태**: ❌ 미구현
- **격차**: 다크 터미널 창(트래픽 라이트 3색 + `tmux: akashic · ~/… (라이브)` 헤더 + `<pre>` 라이브 로그(cargo build/deploy.sh/rc.286/6 domains live)) — 전부 부재. 세션 칩·빠른작업의 `openTerm()` 진입점 죽음.

### 2.17 지침/MCP 에디터 (edOvl)
- **목업**: L1027+ · **라이브**: 부분 (AgentsTab config-chain 으로 대체)
- **상태**: ⚠️ 부분
- **격차**: 목업은 클릭 시 편집 오버레이(edOvl). 라이브는 `AgentsTab` 동적 설정탐지(`cfgrow` 읽기전용 리스트)로 표시만 — 편집 모달 없음. (탐지=plus, 편집=빠짐.)

---

## 3. CSS / 시각 격차 요약

- **완전 누락된 CSS 클래스군**(kakao.css 에 0개 → 해당 화면 렌더 불가):
  - 마켓: `.mkcard .mkcat .mc .mkgrid .mkh .mkn .mkby .mkprice .mkd .mkuse .wallet`
  - 예산: `.budtop .budbtn .budrow .bn .pbar .budamt .budedit .txn .ti .tn .ts .ta`
  - 수익: `.earn-item .earn-top .en .erate .er .earn-stat`
  - 채널: `.chcard .chtop .chav .ch-dc .ch-sl .chn .chs .chbind .bt .chdir .filebox .fileitem .fi .fm`
  - 조직도: `.orgnode .oava .on2 .or .obadge .ob-lead .ob-special .orgindent`
  - 실행이력: `.runrow .rh .rt .rg .rstat .runsteps .rstep .si .runlog`
  - 빌더: `.builder .bl .ctl2 .autoplan-btn .autoplan .stepchips .schip`
  - 터미널: termOvl 인라인 스타일(다크 창) 없음
- **존재하는 CSS**: 셸/대화/위키/흐름카드/프로필 그리드/세그먼트/info 패널 계열은 이식됨(`.kk-*`, `.wfcard .trig .onoff .goal .runline .apvgrid .apvcard .cfgrow .info` 등).
- **시각 톤**: 라이브가 `kk-` 프리픽스 + CSS 변수(`--kk-ink --kk-sub`)로 재작성. 목업의 raw 색상(예 `.wallet` green `#1f9d4d`/`#e6f6ec`, `.mkprice` blue `#2f6bd8`/`#eaf1ff`)은 해당 컴포넌트가 없으니 미반영.

---

## 4. 상태 집계

- **총 목업 화면**: 18개 (메인 4 + 오버레이/모달 14)
- ✅ 일치: **2** (대화방, 위키)
- ⚠️ 부분: **7** (좌측명부+탭, 프로필 agentView, info 패널, 추가 모달, 흐름, 설정, 지침에디터)
- ❌ 미구현: **9** (실행이력, 조직도, 마켓, 지갑·예산, 외부작업상세, 수익, 외부채널, tmux 터미널 미리보기, ↳ 그리고 마켓탭 Placeholder)

> 미구현 9개 중 **마켓**만 탭이고 나머지 8개는 오버레이지만, 그 오버레이들이 마켓/프로필/흐름/설정의 **핵심 클릭 인터랙션**이라 체감 격차가 큼.

---

## 5. 통째로 빠진 화면/기능 (MISSING 전수)

1. **마켓 탭** — Placeholder 만 (마켓 그리드 전체 부재)
2. **지갑·예산 오버레이** (budgetOvl)
3. **공개 에이전트 수익 오버레이** (earningsOvl)
4. **외부 작업 상세 오버레이** (extWorkOvl)
5. **외부 채널 바인딩 오버레이** (channelOvl)
6. **프로젝트 조직도 오버레이** (orgOvl)
7. **실행이력 상세 오버레이** (runOvl)
8. **라이브 tmux 터미널 미리보기** (termOvl)
9. **흐름 빌더 + autoplan(목표 자동구성)** (wfOvl 빌더 영역)
10. **지침/MCP 편집 모달** (edOvl — 탐지만 있고 편집 없음)

---

## 6. 수정 작업 우선순위 권고 (이 감사가 도출)

1. **마켓 탭 네이티브 구현** (Placeholder 제거) — 가장 큰 체감.
2. **흐름 탭 빌더 + autoplan + 프로젝트 그룹핑 + 조직도/실행이력 진입** 추가.
3. **지갑·예산 / 수익 / 외부채널 오버레이** 구현 + 프로필 죽은 `disabled` 버튼을 동작 진입점으로 교체.
4. **tmux 터미널 미리보기 오버레이** + 세션 칩 클릭 배선.
5. **조직도·실행이력 오버레이** + 흐름 카드/세션에서 진입.
6. 마이너: 추가 모달을 목업식 단일화면 그리드로(또는 위저드 유지 결정), info 패널 위치(친구탭 vs 대화탭) 정합, usage $ 표기, gt-sub 실행모드 요약.

> 위 순서대로 처리하면 "목업이랑 너무 다르다"의 주요 원인이 단계적으로 해소됨. 각 항목은 §2 의 해당 화면 절을 fix 스펙으로 사용.
