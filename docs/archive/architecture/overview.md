# OpenXgram — 아키텍처 개요

## 전체 구조

```
                    LLM 인터페이스 레이어
┌──────────────────────────────────────────────────────┐
│  Claude Code   Codex CLI   Gemini CLI   Ollama Bot   │
│      MCP           MCP/stdio   MCP/HTTP    HTTP      │
└──────────────────────┬───────────────────────────────┘
                       │ xgram API (통일 명령)
                       │ send / recv / search / session / secret
┌──────────────────────▼───────────────────────────────┐
│                  xgram daemon (Rust)                  │
│                  단일 바이너리                         │
│                                                      │
│  ┌─────────────────────────────────────────────────┐ │
│  │              Memory Engine                      │ │
│  │  L4 traits ← 야간 reflection                   │ │
│  │  L3 patterns ← NEW/RECURRING/ROUTINE 분류기    │ │
│  │  L2 memories ← 사실·결정·reference·rule        │ │
│  │  L1 episodes ← 세션 묶음                       │ │
│  │  L0 messages ← 원시 + 임베딩 + 서명            │ │
│  └────────────────────┬────────────────────────────┘ │
│                       │                              │
│  ┌────────────────────▼────────────────────────────┐ │
│  │         SQLite + sqlite-vec (store.db)          │ │
│  │         fastembed BGE-small (로컬 임베딩)       │ │
│  └─────────────────────────────────────────────────┘ │
│                                                      │
│  ┌─────────────────────────────────────────────────┐ │
│  │                    Vault                        │ │
│  │  Layer 1: 디스크 AES-256-GCM                  │ │
│  │  Layer 2: 필드 암호화                          │ │
│  │  Layer 3: 전송 암호화 (TLS 1.3 + XMTP)        │ │
│  │  Layer 4: 메모리 zeroize (Phase 2)             │ │
│  │  Layer 5: Shamir 분할 (Phase 2, 옵션)          │ │
│  │  ACL + 머신 화이트리스트 + 일일 한도           │ │
│  │  만료 추적 + 자동 알림                         │ │
│  └─────────────────────────────────────────────────┘ │
│                                                      │
│  ┌─────────────────────────────────────────────────┐ │
│  │             Identity / Keystore                 │ │
│  │  secp256k1 + BIP39 + HD wallet (BIP44)         │ │
│  │  영구 에이전트 / 서브에이전트 자동 파생         │ │
│  │  Tier 0 익명 → Tier 1 친구 → Tier 2 공개      │ │
│  └─────────────────────────────────────────────────┘ │
│                                                      │
│  ┌─────────────────────────────────────────────────┐ │
│  │           Transport Router (자동)               │ │
│  │  1순위: localhost IPC                          │ │
│  │  2순위: Tailscale (mesh)                       │ │
│  │  3순위: XMTP (P2P, 인터넷)                    │ │
│  └─────────────────────────────────────────────────┘ │
│                                                      │
│  ┌────────────────┐  ┌────────────────────────────┐ │
│  │ Discord 어댑터 │  │ Telegram 어댑터            │ │
│  │ 채널 자동 생성 │  │ Setup Agent 1:1            │ │
│  │ Webhook 발신자 │  │ critical 알림              │ │
│  └────────────────┘  └────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
```

## 머신 간 통신

```
  GCP server-main              Mac Mini M4
  ┌──────────────┐             ┌──────────────┐
  │ xgram daemon │ ←Tailscale→ │ xgram daemon │
  │  Akashic     │             │  Claude Code │
  │  Starian     │             │  에이전트들  │
  └──────────────┘             └──────────────┘
         │                           │
         └──────── XMTP ─────────────┘
                (외부 P2P, Tailscale 불가 시)
```

## 데이터 플로우 — 메시지 저장

```
입력 메시지
    │
    ▼
서명 (secp256k1)
    │
    ▼
임베딩 (BGE-small, 로컬)
    │
    ▼
L0 저장 (SQLite)
    │
    ├─ 실시간: memory.delta emit → 공유 정책 적용
    │
    └─ 야간: L0 → L1 통합 → L2 추출 → L3 클러스터 → L4 갱신
```

## 회상 플로우

```
검색 쿼리
    │
    ▼
임베딩 변환 (BGE-small)
    │
    ▼
sqlite-vec ANN 검색
    │
    ▼
복합 점수 계산
α·cosine + β·recency + γ·importance + δ·access_freq
    │
    ▼
Top-K 결과 반환
    │
    ▼
LLM 컨텍스트 주입 (창 크기에 따라 조정)
```

## Phase 1 구현 범위

구현 완료 대상 (5~6주):
- xgram daemon 골격
- Keystore (secp256k1 + BIP39 + HD)
- SQLite + sqlite-vec
- L0 + L1 + L2 메모리 레이어
- Tailscale + XMTP 라우터
- MCP 서버
- TUI (ratatui)
- Discord 어댑터
- Telegram 어댑터
- Vault Layer 1+2
- USDC on Base + OpenAgentX hook

Phase 2 이후:
- L3 + L4
- GUI (Tauri)
- Vault Layer 4+5
- 모바일
