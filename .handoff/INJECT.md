# OpenXgram 컨텍스트 주입 (실행 중 세션용)

당신은 OpenXgram 프로젝트의 다음 작업을 이어받는다. 아래 지시를 즉시 실행하라.

## 1. 정체 (즉시 머릿속에 박을 것)

OpenXgram = 어떤 LLM·머신에서도 동일한 세션·기억·자격증명에 접근하는
기억·자격 인프라(메시징 형태). Akashic의 신체. 마스터 자비스 비전 핵심.

## 2. 필수 컨텍스트 흡수 (지금 즉시)

다음 파일을 순서대로 Read 도구로 읽어라:

1. `.handoff/session-20260503.md` — 마스터 누적 결정 30+개 (가장 중요)
2. `CLAUDE.md` — 프로젝트 지침
3. `docs/prd/PRD-OpenXgram-v1.md` — 21절 메인 PRD
4. `docs/specs/SPEC-memory-transfer-v1.md` — Memory Transfer (1288줄)
5. `docs/specs/SPEC-lifecycle-v1.md` — Lifecycle (1491줄)
6. `docs/checklists/phase-1-mvp.md` — 작업 체크리스트
7. `docs/research/rust-crate-survey-20260503.md` — Rust 크레이트 권고

읽기 완료 후 한 줄로 보고:
"컨텍스트 흡수 완료. 핵심 5개: {정체} / {데이터 디렉토리} / {기술 스택 핵심} / {다음 작업} / {절대 규칙}"

## 3. 절대 규칙 6개 (위반 시 즉시 중단)

- fallback 금지 (모든 오류 raise 또는 명시 로그)
- 표 사용 금지 (목록으로)
- 데이터 디렉토리 ~/.openxgram/ (변경 불가)
- 시간대 KST
- 라이선스 MIT, MVP 후 public
- 롤백 가능 후 자동 승인

## 4. 진행 상태 (2026-05-03 기준)

완료:
- PRD 21절 + SPEC 2종 + 체크리스트 130+ 항목
- 표준 문서 4종 (LICENSE/CONTRIBUTING/CoC/SECURITY)
- Rust 워크스페이스 5 crate (cargo check 통과)
- GitHub OpenXgram/openxgram (PRIVATE) + 10 commits
- GitHub Actions CI 워크플로우
- SSH 키 분리 (id_ed25519_openxgram)

미완료:
- 4개 결정(XMTP REST/multilingual-e5/rusqlite/ChaCha20) SPEC 반영
- silent error 패턴 4개 절대 규칙 추가
- 의존성 실제 추가
- Keystore/DB 실제 구현
- 첫 단위 테스트

## 5. 다음 작업 우선순위

권고 실행 순서:
- D. silent error 절대 규칙 추가 (Pip, 30분)
- A. 4개 결정 SPEC 반영 (Pip, 1시간) — D와 병렬 가능
- B. 의존성 + Keystore 구현 (Eno, 1~2일)
- C. DB 초기화 + 마이그레이션 (Eno, 1일)
- E. Qua 첫 단위 테스트

D+A 병렬 → B → C → E 순.

## 6. 환경·자산

- GitHub: https://github.com/OpenXgram/openxgram (PRIVATE)
- 도메인: openxgram.org (Cloudflare)
- gh CLI 인증됨 (계정 w-partners, admin:org/repo/workflow)
- SSH 키: ~/.ssh/id_ed25519_openxgram (공용 키와 분리)
- 기술 스택: Rust + rusqlite + sqlite-vec + multilingual-e5-small + chacha20poly1305 + reqwest
- 프로젝트 루트: /home/llm/projects/openxgram/

## 7. 다음 행동 (자율 판단)

위 1~6을 흡수 완료한 후, 마스터의 지시 없이도:
- "진행해" → D+A 병렬로 즉시 시작
- 마스터가 다른 지시 주면 그것 우선
- 작업 위임은 Agent 도구로 (Pip/Eno/Qua)

준비되면 다음 한 줄로 보고:
"컨텍스트 주입 완료. {핵심 요약}. 다음 지시 대기 또는 D+A 병렬 시작 준비됨."
