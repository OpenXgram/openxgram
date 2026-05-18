# 05 — 5층 메모리로 에이전트 학습 관리

> 한 줄 요약: L0 messages → L1 episodes → L2 memories → L3 patterns → L4 traits 5 층을 직접 다루며 에이전트의 학습·정체성을 관리한다.

## 시나리오

마스터는 Eno (코딩 에이전트) 가 같은 실수를 반복하지 않게 하고 싶다. 매 세션 끝에 reflection 을 돌려 episode 로 묶고, 핵심 사실·규칙은 L2 memory 로 핀하고, 반복 행동은 L3 patterns 로 분류, 정체성·성향은 L4 traits 로 등록한다. KNN recall 로 과거 유사 컨텍스트를 즉시 꺼낸다.

## 사전 준비

- `xgram init` 완료
- BGE/multilingual-e5-small 임베더 자동 로드 (fastembed, 384d)
- daemon 가동 중이면 야간 reflection cron 자동 실행 (`0 0 15 * * *` UTC = 자정 KST)

## 단계별 명령 시퀀스

```bash
# 1) L0/L1 — 새 session 시작 + 메시지 추가 (자동 임베딩·서명)
xgram session new --title "PRD-NOSTR-08 설계 검토"
xgram session list  # ID 확인

xgram session message \
  --session-id <SID> --sender master \
  --body "relay multi-tenancy 어떻게 격리하지?"
xgram session message \
  --session-id <SID> --sender eno \
  --body "tenant_id prefix + per-tenant rate limit 으로 분리"

# 2) L1 — session 을 episode 로 reflection (요약·키 포인트 추출)
xgram session reflect --session-id <SID>

# 3) 모든 session 일괄 reflection (cron 전 단계)
xgram session reflect-all

# 4) L2 — 핵심 fact / decision / rule 저장
xgram memory add --kind decision \
  --content "relay multi-tenancy = tenant_id prefix + per-tenant rate limit" \
  --session-id <SID>

xgram memory add --kind rule \
  --content "fallback 금지 — 모든 오류는 raise"

# 5) L2 — pin 으로 우선순위 고정 (list 시 상단 노출)
xgram memory list --kind decision
xgram memory pin <MEMORY_ID>
xgram memory unpin <MEMORY_ID>

# 6) L3 — patterns observe (NEW → RECURRING → ROUTINE 자동 분류)
xgram patterns observe --text "테스트 실행 후 lint 돌리기"
xgram patterns observe --text "테스트 실행 후 lint 돌리기"
xgram patterns observe --text "테스트 실행 후 lint 돌리기"
xgram patterns list --classification routine

# 7) L4 — traits 등록 (정체성·성향, manual source)
xgram traits set --key style --value "concise, direct, no fluff"
xgram traits set --key timezone --value "Asia/Seoul"
xgram traits list

# 8) KNN recall — 현재 질문과 유사한 과거 메시지 검색
xgram session recall \
  --query "relay 격리 정책" \
  --k 5
```

## 기대 결과

```
$ xgram session reflect --session-id 01HZ...
✓ reflection 완료
  session_id : 01HZ...
  episodes   : 2 새로 생성
  duration   : 1.3s

$ xgram session recall --query "relay 격리 정책" --k 5
1. (sim 0.91) 01HZ...:msg-3 [eno] tenant_id prefix + per-tenant rate limit 으로 분리
2. (sim 0.84) 01HX...:msg-7 [master] multi-tenancy 어떻게 격리할지...
3. (sim 0.79) ...

$ xgram patterns list --classification routine
- "테스트 실행 후 lint 돌리기"  (count=3, ROUTINE since 2026-05-04 14:33+09:00)
```

## 주의점

- **fallback 금지**: 임베더 로드 실패는 raise. 임베딩 없는 message 저장으로 silently degrade 금지 — BGE 로컬 전용 (PRD §10).
- **롤백 가능**: `memory pin/unpin`, `traits set` 은 즉시 되돌릴 수 있음. `session delete` 는 messages·episodes CASCADE — 신중. memories 는 session_id 가 NULL 로 demote 되어 보존.
- **DB 변경 승인**: 자기 머신 메모리는 자동. 단 `session delete` 는 사실상 destructive — 마스터 명시적 호출만.
- **KST 시간대**: episodes.summary_at, patterns.last_seen, traits.updated_at 모두 Asia/Seoul. 야간 cron 은 자정 KST.
- L3 patterns 분류 임계치(NEW→RECURRING→ROUTINE)는 코드 상수 — 후속 PR 에서 traits 로 노출.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` §10 5 층 메모리 아키텍처
- `docs/prd/PRD-OpenXgram-v1.md` §17 reflection 파이프라인
