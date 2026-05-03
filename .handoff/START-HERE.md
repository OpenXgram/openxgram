# OpenXgram 새 세션 — 시작 가이드

이 폴더에서 새 Claude 세션을 시작했다면 다음 순서로 읽어라.

---

## 1. 컨텍스트 흡수 (필수, 5분)

다음 파일을 순서대로 읽어라. 모두 읽으면 Starian 세션의 7시간 토론 컨텍스트가 95% 복원된다.

1. `.handoff/session-20260503.md` — 핵심 결정 30+개, 진행 상태, 다음 작업 (이 파일이 핵심)
2. `docs/prd/PRD-OpenXgram-v1.md` — 21절 메인 PRD
3. `docs/specs/SPEC-memory-transfer-v1.md` — Memory Transfer 사양 (1288줄)
4. `docs/specs/SPEC-lifecycle-v1.md` — Lifecycle 사양 (1491줄)
5. `docs/checklists/phase-1-mvp.md` — Phase 1 작업 체크리스트 (130+ 항목)
6. `docs/research/rust-crate-survey-20260503.md` — Rust 크레이트 권고
7. `CLAUDE.md` — 이 프로젝트 에이전트 지침

---

## 2. 핵심 절대 규칙 6개

읽기 전에 머릿속에 박아라:
- fallback 금지 — 모든 오류 raise 또는 명시 로그. `unwrap_or_default()` 금지
- 표 사용 금지 — 목록으로만
- 데이터 디렉토리 ~/.openxgram/ — 변경 불가
- 시간대 KST — 모든 타임스탬프 Asia/Seoul
- 라이선스 MIT, MVP 후 public — 현재 private
- 롤백 가능 후 자동 승인 — 되돌릴 수 없는 작업은 마스터 승인 필수

---

## 3. 다음 작업 우선순위

권고 순서 (D+A 병렬 가능):

- D. Pip에 silent error 패턴 절대 규칙 14절 추가 (반나절)
- A. Pip에 4개 결정 SPEC 반영 — XMTP REST / multilingual-e5 / rusqlite / ChaCha20-Poly1305 (반나절)
- B. Eno에 의존성 추가 + Keystore 구현 — k256/bip39/rusqlite/sqlite-vec/fastembed/chacha20poly1305/reqwest (1~2일)
- C. Eno에 DB 초기화 + 첫 마이그레이션 (1일)
- E. Qua에 첫 단위 테스트 작성

D+A를 Pip에게 동시 요청 → B → C → E 순으로 진행.

---

## 4. 마스터 지시 응답 방식

마스터가 한 줄 지시하면:
- 의도 파악 → 어느 에이전트에 위임할지 결정 (Pip/Eno/Qua/Res)
- 위임은 Agent 도구로 (이 세션은 오케스트레이션)
- 직접 코드 수정은 가급적 피함
- 코딩은 반드시 worktree isolation으로 실행

---

## 5. 모르는 것 발견 시

1. `.handoff/session-20260503.md` 검색
2. `docs/` 검색
3. 그래도 없으면 마스터에게 한 줄 질문 (추측 답변 금지)

---

## 6. 환경 정보

- 프로젝트 루트: /home/llm/projects/openxgram/
- GitHub: https://github.com/OpenXgram/openxgram (PRIVATE)
- GitHub 계정: w-partners (gh CLI 인증)
- SSH 키: ~/.ssh/id_ed25519_openxgram
- 도메인: openxgram.org (Cloudflare, Phase 2 DNS 설정 예정)
- 빌드: cargo check 통과, git 9개 커밋 누적
- 임베딩 모델: multilingual-e5-small (한국어 최적화)
- 암호화: ChaCha20-Poly1305
- DB: rusqlite (SQLite + sqlite-vec)

---

## 7. 시작

위 1번 파일들 읽은 후 마스터에게 한 줄:

"컨텍스트 흡수 완료. {다음 작업} 시작합니다."

---

## 8. 이미 기동 중인 claude 세션에 컨텍스트만 주입할 때

새 세션 시작이 아니라 이미 떠 있는 claude에 컨텍스트만 넣고 싶을 때:

방법 A — 짧은 명령:
  claude 입력창에 다음 한 줄:
  > @.handoff/INJECT.md 읽고 그대로 실행

방법 B — 클립보드 자동 복사:
  $ ./.handoff/inject.sh
  클립보드에 컨텍스트 주입 프롬프트 복사됨
  -> claude 창으로 가서 Cmd+V -> 엔터

방법 C — alias 등록 (편의):
  ~/.bashrc 또는 ~/.zshrc에:
  alias xginject='/home/llm/projects/openxgram/.handoff/inject.sh'

  사용:
  $ xginject  # 어디서든 복사
  -> claude 창에 Cmd+V

이 방법은 OpenXgram MVP가 자동화할 작업의 1차 수동 시뮬레이션.
Phase 1 완성 후엔: $ xgram session push --to claude-current
