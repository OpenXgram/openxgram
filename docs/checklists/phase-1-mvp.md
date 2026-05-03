# Phase 1 MVP 체크리스트

버전: v0.1.0.0-alpha.1
목표: 5~6주 내 핵심 기능 동작

## 기반 인프라

- [ ] Rust 워크스페이스 초기화 (`crates/` 구조)
- [ ] xgram daemon 골격 (tokio async runtime)
- [ ] --daemon / --tui / --headless 모드 분기
- [ ] 설정 파일 (`~/.xgram/config.toml`)
- [ ] 로깅 (tracing crate)

## 신원 / Keystore

- [ ] secp256k1 키페어 생성
- [ ] BIP39 시드 + BIP44 HD 파생 구현
- [ ] 영구 에이전트 키 수동 발급 CLI
- [ ] 서브에이전트 키 자동 파생 (`m/44'/60'/parent'/0/task_seq`)
- [ ] keystore 암호화 저장 (AES-256-GCM)

## 저장소

- [ ] SQLite DB 초기화 (`store.db`)
- [ ] sqlite-vec extension 로드
- [ ] fastembed BGE-small 모델 로드 (로컬)
- [ ] L0 messages 테이블 + 임베딩 컬럼
- [ ] L1 episodes 테이블
- [ ] L2 memories 테이블 (fact/decision/reference/rule)
- [ ] 회상 복합 점수 쿼리 (α·β·γ·δ)

## Vault

- [ ] Layer 1: 디스크 암호화 (AES-256-GCM)
- [ ] Layer 2: 필드 암호화
- [ ] ACL 구조 (에이전트별 권한)
- [ ] 만료 추적 + 7일 전 알림
- [ ] `xgram.secret` 명령 구현

## Transport

- [ ] localhost IPC 구현 (Unix socket)
- [ ] Tailscale 라우터 연결
- [ ] XMTP 어댑터 연결
- [ ] 자동 라우팅 (IPC → Tailscale → XMTP)

## MCP 서버

- [ ] MCP 서버 골격 (`xgram serve --mcp`)
- [ ] xgram.send 도구
- [ ] xgram.recv 도구
- [ ] xgram.search 도구
- [ ] xgram.session 도구
- [ ] xgram.secret 도구

## TUI

- [ ] ratatui 기반 TUI 골격
- [ ] 메시지 목록 화면
- [ ] 기억 검색 화면
- [ ] Vault 상태 화면
- [ ] 세션 상태 화면

## Discord 어댑터

- [ ] Discord Bot 연결
- [ ] 에이전트당 채널 자동 생성
- [ ] Webhook 기반 발신자 분리 (모델 C)
- [ ] 알림 전송 기능

## Telegram 어댑터

- [ ] Setup Agent 1:1 채팅 구현
- [ ] critical 알림 전송 기능
- [ ] Vault 만료 알림 연동

## 세션 이동성

- [ ] `xgram session push <target>` 구현
- [ ] `xgram session pull <source>` 구현
- [ ] 첨부파일 해시 기반 중복 제거

## 결제

- [ ] USDC on Base 송금 구현
- [ ] OpenAgentX 어댑터 hook (숨김)

## 검증

- [ ] 시나리오 A: 에이전트 간 기억 공유 + 검증
- [ ] 시나리오 B: Vault 키 자동 공유 + 만료 알림
- [ ] 시나리오 C: 세션 이동 GCP → Mac Mini
- [ ] 시나리오 D: NEW/ROUTINE 분류 (Phase 2 대상, 기초 준비)
- [ ] 시나리오 E: 파일 송수신 무결성
- [ ] 시나리오 F: ChatGPT 웹 토론 → 사이드카 import → Claude Code attach
- [ ] 시나리오 G: 웹 ChatGPT ↔ 사이드카 ↔ 웹 Claude 중계 (컨텍스트 운반자 역할 입증)

---

## Memory Transfer Phase 1 MVP (5~6일)

### 데이터 모델 (0.5일)

- [ ] transfer_logs 테이블 정의
- [ ] webhook_endpoints 테이블 정의
- [ ] webhook_acl 테이블 정의
- [ ] 마이그레이션 SQL 작성 + 적용 절차

### Push (Send Out) (1.5일)

- [ ] 메모리 추출기 (범위 선택: session/recent/pin/tag/search)
- [ ] Text Package 빌더 (Markdown + JSON)
- [ ] 단일 .md 파일 생성기
- [ ] 보안 필터: secret/vault 태그 제외
- [ ] 보안 필터: 키 패턴 마스킹 (API key/token/seed words)
- [ ] --preview 플래그
- [ ] 클립보드 출력 (linux: xclip, macos: pbcopy, ssh: OSC52)
- [ ] Discord 채널 백업 (Webhook으로 #xgram-backup)

### Pull (Receive) (1일)

- [ ] 입력 파서 (markdown frontmatter, JSON, yaml 자동 감지)
- [ ] 스키마 검증 (JSON Schema)
- [ ] 중복 감지 (서명 hash 기반)
- [ ] 세션 매핑: 새 세션 / 현재 세션 / 특정 세션 ID
- [ ] L0/L1/L2 자동 분배
- [ ] 임베딩 생성 후 저장

### 양방향 인터페이스 (1일)

- [ ] CLI: xgram extract
- [ ] CLI: xgram backup-push
- [ ] CLI: xgram session import
- [ ] CLI: xgram webhook list/add (placeholder)
- [ ] MCP 도구: xgram.transfer.push
- [ ] MCP 도구: xgram.transfer.pull
- [ ] MCP 도구 JSON Schema 정의

### TUI 페이지 (1일)

- [ ] Memory Transfer 페이지 진입
- [ ] 좌측: Push 옵션 트리 (범위/형식/대상)
- [ ] 우측: Pull 입력 영역 (붙여넣기/파일 드래그)
- [ ] 미리보기 모달
- [ ] 키바인딩 정의 (Spec 7.1 참조)
- [ ] 상태 표시 (전송 중/완료/오류)

### 보안·감사 (0.5일)

- [ ] audit_log 기록 (모든 outbound)
- [ ] Rate limit (시간/일 단위)
- [ ] 마스터 승인 정책 (auto/confirm/mfa) — Phase 1은 confirm만
- [ ] 검증 실패 시 즉시 raise (fallback 금지)

### 테스트 (0.5일)

- [ ] 단위 테스트 (추출기, 빌더, 파서, 마스킹)
- [ ] 통합 시나리오 (Push → 클립보드 → 다른 머신 Pull)
- [ ] 보안 케이스 (큰 payload, 잘못된 형식, 마스킹 누락 검증)

### 문서 (0.5일)

- [ ] CLI 사용법 (docs/usage/memory-transfer.md)
- [ ] TUI 가이드
- [ ] 트러블슈팅
