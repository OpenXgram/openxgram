# PRD — OpenXgram v1

작성일: 2026-04-30 (KST)
버전: v0.1.0.0-alpha.1
상태: 초안 (마스터 결정 반영)
작성자: Eno (agt_dept_engineering) — 마스터 결정 누적 정리

---

## 0. 정체

OpenXgram은 기억·자격 인프라다. 메시지는 표면 표현이고 본질은 메모리와 신원 관리다.

- 한 줄 정의: "어떤 LLM·머신에서도 동일한 세션·기억·파일·자격증명에 접근할 수 있는 기억·자격 인프라"
- Akashic 에이전트의 신체로서, 5층 메모리 아키텍처와 Vault를 통해 에이전트들의 지식과 비밀을 영구 보관·이동·공유한다
- 메시지는 이 인프라가 제공하는 표면 인터페이스다

---

## 1. 핵심 3원칙

- 터미널 연결 쉬움: 1줄 attach (`xgram attach --agent <id>`)
- 영구 연결: 24/7 사이드카 데몬, 오프라인 중에도 메시지 큐 유지
- 세션 이동성: 어디서든 이어가기 + 기억 송수신

---

## 2. 사이드카 모델

- 머신마다 1개 데몬 (Rust, 단일 바이너리 `xgram`)
- 4가지 실행 모드:
  - `xgram --daemon` — 백그라운드 데몬
  - `xgram --tui` — TUI 인터페이스
  - `xgram --gui` — Tauri GUI (Phase 2)
  - `xgram --headless` — CI/봇 환경용
- 머신마다 N개 에이전트 키를 keystore에서 관리
- 데몬은 systemd / launchd / 수동 프로세스 중 환경에 맞게 구동

---

## 3. 신원 체계 (블록체인 강도 2)

### 키 유형

- secp256k1 / EVM 호환 키페어 (Base 체인)
- BIP39 시드 + HD wallet 파생 (BIP44 경로: `m/44'/60'/0'/0/N`)

### 에이전트 종류별 키 발급

- 영구 에이전트: 수동 발급 (마스터 승인)
- 서브에이전트: 옵션 B 자동 발급
  - HD 파생 경로: `m/44'/60'/parent_index'/0/task_seq`
  - 부모 키에서 파생 → 부모 서명으로 신원 증명 가능
- 헤드리스 봇: 자동 발급

### 신뢰 등급

- Tier 0 — 익명 (키 있음, 등록 없음)
- Tier 1 — 친구등록 (로컬 신뢰 목록에 등록)
- Tier 2 — OpenAgentX 등록 (공개 검색 가능)

---

## 4. Transport 자동 라우팅

라우팅 우선순위 (자동, 설정 불필요):

- 1순위: localhost IPC — 같은 머신 내 에이전트
- 2순위: Tailscale — 같은 Tailnet의 다른 머신
- 3순위: XMTP — 외부 P2P (인터넷)

어댑터 (기본 활성화):
- Discord 어댑터 — 기본 ON, 채널 자동 생성, Webhook으로 발신자 분리
- Telegram 어댑터 — 기본 ON, 마스터 ↔ Setup Agent 1:1 + critical 알림

---

## 5. 저장 + 검색

### 저장소

- SQLite + sqlite-vec (전부 한 파일, `~/.xgram/store.db`)
- 임베딩 모델: BGE-small (fastembed, 로컬 전용)
- **fallback 금지** — 임베딩 실패 시 raise, 조용히 생략하지 않음

### 회상 점수

복합 점수로 순위 결정:

```
score = α·cosine_similarity
      + β·recency_decay
      + γ·importance_weight
      + δ·access_frequency
```

α, β, γ, δ는 설정 가능 (기본값 TBD, Phase 1에서 경험적으로 결정)

---

## 6. Memory 5층 아키텍처 (핵심)

```
L4  traits    ← 정체성·성향 (야간 reflection 도출)
L3  patterns  ← NEW / RECURRING / ROUTINE 분류기
L2  memories  ← 사실·결정·reference·rule (핀 가능)
L1  episodes  ← 세션 단위 묶음
L0  messages  ← 원시 메시지 + 임베딩 + 서명
```

### L0 messages

- 필드: id, session_id, agent_id, role, content, embedding, signature, timestamp, metadata
- 모든 메시지는 발신자 키로 서명

### L1 episodes

- 세션 단위로 메시지를 묶음
- session_id, start_time, end_time, summary, participant_ids

### L2 memories

- 타입: fact / decision / reference / rule
- 핀(pin) 가능 — 회상 시 상위 노출
- source_episode_id로 출처 추적

### L3 patterns

- 임베딩 거리 기반 클러스터링 (온라인 + 야간 재학습)
- 분류: NEW / RECURRING / ROUTINE
- 마스터 피드백 루프로 임계값 자동 조정:
  - `/this-is-different` — NEW로 강제 분류
  - `/merge-with <pattern_id>` — 기존 패턴과 병합
- TUI·GUI·Discord·Telegram으로 알림

### L4 traits

- 에이전트의 정체성·성향 요약
- 야간 reflection으로 L2·L3에서 자동 도출
- 수동 편집 가능 (마스터 승인)

### 야간 Reflection

- 매일 자정 KST
- 오늘의 L0 → L1 통합 → L2 추출 → L3 클러스터 업데이트 → L4 갱신

---

## 7. NEW / RECURRING / ROUTINE 분류기

### 분류 기준

- NEW: 가장 가까운 기존 패턴과 cosine distance > threshold_new
- RECURRING: threshold_recurring < distance ≤ threshold_new
- ROUTINE: distance ≤ threshold_recurring

### 임계값 관리

- 초기값: threshold_new=0.5, threshold_recurring=0.2
- 마스터 피드백 수신 시 자동 조정 (+/- 0.05)
- 야간 재학습 시 클러스터 중심 갱신

### 알림 채널

- TUI / GUI / Discord / Telegram 중 설정된 채널로 발송

---

## 8. Vault — 자격증명 인프라

### 암호화 레이어

- Layer 1: 디스크 전체 암호화 (AES-256-GCM, 키는 keystore에 보관)
- Layer 2: 필드 수준 암호화 (민감 필드 개별 암호화)
- Layer 3: 전송 암호화 (TLS 1.3 + XMTP 암호화)
- Layer 4: 메모리 zeroize (Phase 2 — 메모리 상주 비밀 제거)
- Layer 5: Shamir 분할 (Phase 2 — 옵션, 임계값 기반 복구)

### ACL

- 에이전트별 접근 권한 설정
- role 기반 권한 (reader / writer / admin)
- 머신 화이트리스트 (특정 머신에서만 접근 가능)
- 일일 접근 한도 (rate limiting)

### 마스터 승인 정책 3단계

- auto — 자동 허용 (낮은 위험 작업)
- confirm — 마스터 확인 필요
- mfa — MFA 추가 인증 필요

### 자동 Sync

- share_policy 재사용 (메모리 공유 정책과 동일 구조)
- 머신 간 Vault sync는 마스터 승인 후 활성화

### 만료·갱신 추적

- key-registry.md 29개 서비스 자동 마이그레이션 대상
- 만료 N일 전 알림 (기본 7일)
- Instagram/Threads: 60일 주기 갱신 자동 추적

---

## 9. 메모리 공유 정책

각 에이전트는 공유 정책을 설정한다:

- direction: push / pull / both / none
- scope: all / project:\* / tag:\*
- 실시간 공유: memory.delta 메시지 emit
- 예: Akashic이 다른 에이전트에게 특정 기억을 push

---

## 10. 세션 이동성

### 명령

- `xgram session push <target>` — 현재 세션을 다른 머신/에이전트로 전송
- `xgram session pull <source>` — 다른 머신의 세션 가져오기
- `xgram session sync` — 양방향 동기화

### 운반 항목

- 메시지 (L0)
- 기억 (L2)
- 임베딩 벡터
- 첨부파일 (해시 기반 중복 제거)

### 신원 연속성

- HD 키 동일 → 어느 머신이든 같은 신원으로 attach
- 부모 키 서명으로 세션 소유권 증명

---

## 11. 다른 LLM 호환

### 어댑터

- Claude Code: MCP 서버 (`xgram serve --mcp`)
- Codex CLI: MCP / stdio
- Gemini CLI: MCP / HTTP
- Ollama: HTTP + tools API
- 사용자 봇: HTTP REST

### 노출 명령 통일 (모든 어댑터 공통)

- `xgram.send(to, content, metadata)` — 메시지 전송
- `xgram.recv(from, limit)` — 메시지 수신
- `xgram.search(query, top_k)` — 기억 검색
- `xgram.session(action)` — 세션 관리
- `xgram.secret(action, key)` — Vault 접근

### 컨텍스트 자동 주입

- LLM 능력(context window 크기)에 따라 자동 조정:
  - 작은 창: Top-K 요약만 주입
  - 큰 창: 전체 에피소드 주입

---

## 12. 결제

- 토큰: USDC on Base
- 결제 통합: OpenAgentX 어댑터 (숨김, 내부 처리)
- 마스터 = OpenAgentX 관리자 (정책 직접 정의)
- 계정 구조: 한 Google 계정 → 여러 키페어(에이전트) 묶음

---

## 13. 사람 인터페이스

### TUI (Phase 1 — 기본 제공)

- ratatui 기반
- 기능: 메시지 목록, 기억 검색, Vault 관리, 세션 상태

### GUI (Phase 2)

- Tauri + React
- TUI와 동일 기능 + 시각적 메모리 그래프

### Discord 어댑터 (Phase 1 — 기본 ON)

- 채널 자동 생성 (에이전트당 1채널)
- Webhook으로 발신자 분리 (모델 C)
- 에이전트 → Discord 채널로 알림/보고

### Telegram 어댑터 (Phase 1 — 기본 ON)

- 마스터 ↔ Setup Agent 1:1 채팅
- critical 알림 (Vault 만료, 패턴 NEW 감지 등)

---

## 14. 절대 규칙 (마스터 확정)

- fallback 금지: 모든 오류는 raise 또는 명시 로그. 조용히 넘어가지 않는다
- 롤백 가능 후 자동 승인: 되돌릴 수 없는 작업은 마스터 승인 필수
- DB 변경은 마스터 승인: SELECT 자유, INSERT/UPDATE/DELETE 승인 필수
- 시간대 KST: 모든 타임스탬프 Asia/Seoul 기준
- 표 사용 금지: 보고서·문서 모두 목록으로
- 서브에이전트 디스코드 가시성: 작업 시작·완료 시 디스코드 보고 필수

---

## 15. MVP 범위 (Phase 1) — 추정 5~6주

구현 항목:
- 사이드카 골격 + Keystore + SQLite + sqlite-vec
- L0 messages + L1 sessions + L2 memories
- Tailscale + XMTP transport (자동 라우팅)
- MCP 서버 + CLI (`xgram`)
- TUI (ratatui)
- Discord 어댑터 (기본 ON, 채널 자동 생성)
- Telegram 어댑터 (Setup Agent 1:1)
- Vault 기본 (Layer 1+2 암호화, ACL, 만료 추적)
- HD 키페어 (영구 + 서브에이전트 자동 파생)
- USDC on Base 송금 + OpenAgentX 어댑터 hook
- 회상 복합 점수 (α·관련성 + β·최신성 + γ·중요도 + δ·접근빈도)
- Memory Transfer Phase 1 MVP (Text Package + 클립보드 + Discord 백업 + 기본 Pull, 약 5~6일)

---

## 16. Phase 2+ (후속)

- L3 패턴 클러스터링 + NEW/RECURRING/ROUTINE 분류기
- L4 traits 추출 + 야간 reflection
- GUI (Tauri + React)
- Vault Layer 4+5 (메모리 zeroize, Shamir 분할)
- 모바일 (Tauri Mobile)
- IPFS 큰 파일 저장
- Codex CLI / Gemini CLI 어댑터
- Memory Transfer Phase 1.5 (Email + Telegram + Webhook outbound + 코드 추출)
- Memory Transfer Phase 2 (Inbound webhook + GUI 페이지 + 모든 추출 형식)
- Memory Transfer Phase 2+ (브라우저 확장, 클립보드 동기화 자동화)

---

## 17. Memory Transfer (양방향 기억 전이)

용도:
- 웹 LLM(ChatGPT/Claude.ai/Gemini Web)과 사이드카 사이 기억 전이 (사람이 다리)
- 외부 시스템(Notion/Linear/Slack 등)과 자동 통합 (webhook 양방향)
- 백업 (사이드카 다 죽어도 외부 4채널에 살아있음)

방향:
- Push (Send Out): 사이드카 → 외부
- Pull (Receive): 외부 → 사이드카

추출 형식 4종:
- Text Package (Markdown + JSON 본체) — 토론·검토용
- 단일 파일 (.md / .json / .yaml) — 첨부·아카이브
- 코드 추출 (.py / .ts / .sql / .nginx / .conf) — 결정·설정 → 실행 코드
- Webhook Payload — 자동 송수신용 (서명 + 타임스탬프)

전송 통로 4종:
- 클립보드 (수동, 가장 빠름)
- 이메일 (SMTP)
- Telegram (@starianbot 1:1)
- Discord (봇 + Webhook, 이미 운영 중 12+ 채널)

워크플로우:
- Push: 선택(범위) → 형식 선택 → 대상 선택 → 보안 검증 → 발송
- Pull: 입력(붙여넣기/파일/HTTP POST) → 파싱·검증 → 세션 매핑 → 임베딩

자동 트리거:
- on session-end
- on pin
- on schedule (cron)
- on memory-merge

보안:
- 태그 secret/vault 자동 제외
- 키 패턴 자동 마스킹
- --preview 기본
- HMAC + 시간 윈도우 (inbound)
- 화이트리스트 (IP + 도메인 + 키페어)
- 1MB payload 상한
- 외부 발송 명시 승인 (fallback 금지, 모든 검증 실패 즉시 raise)

상세 사양: docs/specs/SPEC-memory-transfer-v1.md (Pip 작성)

---

## 18. 위험 요소

- Discord 정책 변경: Bot을 통한 Webhook 발신자 분리가 Discord TOS 해석에 따라 제한될 수 있음. 모니터링 필요.
- XMTP 메인넷 전환: XMTP가 testnet → mainnet으로 전환 중. 프로토콜 변경 시 어댑터 업데이트 필요.
- BGE-small 한국어 임베딩 품질: 한국어 텍스트의 임베딩 품질이 영어 대비 낮을 수 있음. Phase 1에서 실측 후 모델 교체 여부 결정.
- OpenAgentX API 안정성: OpenAgentX가 내부 프로젝트이므로 API 변경 시 어댑터 영향. 인터페이스 추상화 필수.
- Memory Transfer Outbound 데이터 유출 위험: 마스킹 + 태그 제외 + 미리보기 + 감사 로그 4중 방어
- Inbound webhook 악의 페이로드: 서명 검증 + 화이트리스트 + 크기 상한 + JSON Schema

---

## 19. 검증 시나리오 (마스터 확정)

### 시나리오 A — 에이전트 간 기억 공유 + 검증 요청

1. 에이전트 A가 결정 사항을 L2 memory에 저장
2. 에이전트 A가 에이전트 B에게 memory.delta 메시지 push
3. 에이전트 B가 `xgram.search("해당 결정")`으로 회상 성공
4. 에이전트 B가 에이전트 A에게 검증 결과 응답
5. 기대: 에이전트 B가 에이전트 A의 기억을 그대로 읽고 서명 검증 통과

### 시나리오 B — Vault 키 자동 공유

1. Vault에 Instagram API 키 저장 (만료일 포함)
2. Flowsync 에이전트가 `xgram.secret("get", "instagram_token")` 요청
3. ACL 확인 → Flowsync 에이전트 권한 있음 → 키 반환
4. 70일 후 만료 7일 전 알림 자동 발송 (Telegram)
5. 기대: 만료 추적 + ACL 적용 + 알림 정상 동작

### 시나리오 C — 세션 이동 (GCP → Mac Mini)

1. GCP에서 에이전트가 대화 세션 진행
2. `xgram session push macmini` 실행
3. Mac Mini에서 `xgram session pull gcp` 실행
4. Mac Mini에서 `xgram attach` 후 이어서 대화
5. 기대: 메시지·기억·임베딩 모두 동일하게 복원, 신원 연속성 유지

### 시나리오 D — NEW / ROUTINE 자동 분류

1. 에이전트가 매일 같은 시스템 상태 점검 루틴 실행 (10회 반복)
2. 11번째 실행 시 L3 분류기가 ROUTINE으로 분류
3. 마스터가 `/this-is-different` 피드백
4. 분류기가 임계값 재조정 → NEW로 재분류
5. 기대: 자동 분류 정확도 + 피드백 반영

### 시나리오 E — 파일 송수신

1. 에이전트 A가 5MB 파일을 `xgram.send(to=B, file=...)` 전송
2. Transport 라우터가 Tailscale 경로 선택
3. 에이전트 B가 `xgram.recv()` 후 파일 수신
4. 파일 해시 비교로 무결성 검증
5. 기대: 파일 손실 없음, 해시 일치, 중복 전송 방지

---

### 시나리오 F — ChatGPT 웹 토론 → 사이드카 import → Claude Code attach (마스터의 핵심 요구)

1. 마스터가 ChatGPT 웹에서 중요한 토론·결정 세션 진행
2. Memory Transfer Push로 Text Package(.md) 생성 → 클립보드 복사
3. `xgram session import` 명령으로 사이드카에 Pull (붙여넣기)
4. Claude Code에서 `xgram attach` 후 해당 기억을 `xgram.search()`로 회상
5. 기대: 웹 LLM 세션이 사이드카로 완전 이전, Claude Code에서 컨텍스트 연속성 유지

### 시나리오 G — 웹 ChatGPT ↔ 사이드카 ↔ 웹 Claude 중계

1. ChatGPT 웹 대화 중 "OpenXgram 형식으로 정리" 요청 → JSON 받기
2. 사이드카에 import → 세션 생성
3. 사이드카에서 Memory Transfer → Claude.ai용 Text Package 추출
4. Claude 웹에 붙여넣기 → 그 컨텍스트로 대화 이어가기
5. 기대: ChatGPT와 Claude가 같은 결정·미해결 질문·파일 컨텍스트 공유. 사이드카가 웹 LLM 사이 컨텍스트 운반자 역할 입증.

---

## 20. 다음 단계

- Pip에게 인터페이스 상세 사양 작성 위임 (xgram CLI 명령 체계, MCP 도구 스펙, DB 스키마 초안)
- Res에게 XMTP SDK·fastembed·sqlite-vec 최신 API 리서치 위임
- Eno에게 Rust 워크스페이스 초기화 위임 (Phase 1 구현 시작 — 마스터 승인 후)
