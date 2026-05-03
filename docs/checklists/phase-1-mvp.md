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
