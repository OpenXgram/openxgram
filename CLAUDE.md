> **새 세션 시작 시**: `.handoff/START-HERE.md` 먼저 읽기. 7시간 토론 컨텍스트가 자동 복원됨.
> **이미 기동 중인 claude에 컨텍스트만 주입**: `@.handoff/INJECT.md 읽고 그대로 실행` 한 줄

# OpenXgram — 에이전트 작업 지침

## 프로젝트 정체

OpenXgram은 기억·자격 인프라다. 메시지는 표면 표현이고 본질은 메모리와 신원 관리다.
Akashic의 신체. 어떤 LLM·머신에서든 동일한 세션·기억·파일·자격증명에 접근 가능하게 한다.

## 5층 메모리 아키텍처 (핵심)

```
L4  traits    ← 정체성·성향 (야간 reflection 도출)
L3  patterns  ← NEW / RECURRING / ROUTINE 분류기
L2  memories  ← 사실·결정·reference·rule (핀 가능)
L1  episodes  ← 세션 단위 묶음
L0  messages  ← 원시 메시지 + 임베딩 + 서명
```

저장소: SQLite + sqlite-vec (단일 파일)
임베더: BGE-small (fastembed, 로컬 전용, fallback 없음)

## Vault

- 다층 암호화: 디스크 + 필드 + 전송 + 메모리 zeroize
- ACL: 에이전트별/role별, 머신 화이트리스트, 일일 한도
- 마스터 승인 정책 3단계: auto / confirm / mfa
- key-registry.md 29개 서비스 마이그레이션 대상

## 절대 규칙 (6개)

1. **fallback 금지** — 모든 오류는 raise 또는 명시 로그. 조용히 넘어가지 않는다.
2. **롤백 가능 후 자동 승인** — 되돌릴 수 없는 작업은 마스터 승인 필수.
3. **DB 변경은 마스터 승인** — SELECT는 자유, INSERT/UPDATE/DELETE는 승인 필수.
4. **시간대 KST** — 모든 타임스탬프 Asia/Seoul 기준.
5. **표(table) 사용 금지** — 보고서·문서 모두 목록으로.
6. **서브에이전트 디스코드 가시성** — 작업 시작·완료 시 디스코드 보고 필수.

## 디렉토리 가이드

```
docs/prd/          ← PRD 문서 (Pip 담당)
docs/architecture/ ← 아키텍처 다이어그램
docs/decisions/    ← ADR (엔지니어링 결정 기록)
docs/checklists/   ← Phase별 체크리스트
agents/pip/        ← PRD 작성 에이전트
agents/eno/        ← 구현 에이전트 (Rust 코어)
agents/qua/        ← 검증 에이전트
agents/res/        ← 리서치 에이전트
crates/            ← Rust 워크스페이스 (Phase 1 구현 시작)
ui/                ← TUI (ratatui) + GUI (Tauri, Phase 2)
scripts/           ← 빌드·배포 스크립트
```

## 버전 관리

version.json + package.json 동기화 필수.
BUILD는 CI/CD 자동 증가 — 수동 변경 금지.
현재: v0.1.0.0-alpha.1

## 언어·기술 스택

- 코어 데몬: Rust (단일 바이너리)
- TUI: ratatui
- GUI: Tauri + React (Phase 2)
- DB: SQLite + sqlite-vec
- 임베딩: fastembed (BGE-small)
- 암호화: secp256k1, BIP39, HD wallet
- Transport: IPC → Tailscale → XMTP
- 결제: USDC on Base

## 에이전트 역할 분담

- Pip: PRD·사양 문서 작성
- Eno: Rust 코어 구현
- Qua: 테스트·검증
- Res: 외부 라이브러리·프로토콜 리서치
