# OpenXgram

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/OpenXgram/openxgram/blob/main/LICENSE)
![CI](https://github.com/OpenXgram/openxgram/actions/workflows/ci.yml/badge.svg)
[![Version](https://img.shields.io/badge/version-0.1.0--alpha.1-blue)](https://github.com/OpenXgram/openxgram/blob/main/version.json)

**Repository**: https://github.com/OpenXgram/openxgram  
**Homepage**: https://openxgram.org

**OpenXgram — Memory & Credential Infrastructure for Multi-Agent, Multi-Machine, Multi-LLM**

OpenXgram은 어떤 LLM·머신에서든 동일한 세션·기억·파일·자격증명에 접근할 수 있게 해주는 기억·자격 인프라다. 메시지는 표면 표현이고 본질은 메모리와 신원 관리다. Akashic 에이전트의 신체로서, 5층 메모리 아키텍처와 Vault를 통해 에이전트들의 지식과 비밀을 영구 보관·이동·공유한다. 머신마다 경량 사이드카 데몬 하나를 두고 Tailscale → XMTP 자동 라우팅으로 P2P 연결하며, secp256k1 HD 키페어 기반 신원으로 어디서든 같은 에이전트로 attach할 수 있다. 결제는 USDC on Base, OpenAgentX 통합을 통해 에이전트 경제에 연결된다.

```
┌─────────────────────────────────────────────────────┐
│                    OpenXgram                        │
│                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────┐  │
│  │  Claude  │  │  Codex   │  │  Gemini / Ollama │  │
│  └────┬─────┘  └────┬─────┘  └────────┬─────────┘  │
│       │             │                 │              │
│       └─────────────┴─────────────────┘              │
│                     │ MCP / HTTP                      │
│            ┌────────▼────────┐                       │
│            │   xgram daemon  │ ← sidecar (Rust)      │
│            │  ┌───────────┐  │                       │
│            │  │  L0 msg   │  │                       │
│            │  │  L1 ep    │  │                       │
│            │  │  L2 mem   │  │  SQLite + sqlite-vec  │
│            │  │  L3 pat   │  │                       │
│            │  │  L4 trait │  │                       │
│            │  └───────────┘  │                       │
│            │  ┌───────────┐  │                       │
│            │  │   Vault   │  │ ← 암호화 자격증명     │
│            │  └───────────┘  │                       │
│            └────────┬────────┘                       │
│          IPC │  Tailscale │  XMTP                    │
│         (local)  (mesh)   (external)                 │
└─────────────────────────────────────────────────────┘
```

## 빠른 시작 (Phase 1)

### 빌드

```bash
git clone https://github.com/OpenXgram/openxgram
cd openxgram
cargo build --release                       # 기본 빌드 (DummyEmbedder)
cargo build --release --features fastembed  # multilingual-e5-small 의미 임베딩
                                            # (pkg-config + libssl-dev 필요)
```

### 사용 흐름 — 머신 한 대 설치

```bash
export XGRAM_KEYSTORE_PASSWORD='12자이상-안전한패스워드'

# 1. 비대화 init — 데이터 디렉토리 + DB + master 키페어 + manifest 생성
xgram init --alias gcp-main --role primary

# 2. 환경 진단
xgram doctor                # 사람용 출력
xgram doctor --json         # 다른 도구 통합

# 3. 데몬 시작 (foreground 또는 systemd user unit)
xgram daemon                                          # foreground
xgram daemon-install --binary $(which xgram)          # systemd unit 생성
systemctl --user enable --now openxgram-sidecar       # 활성화

# 4. session 작업
xgram session new --title "research-thread"
xgram session message --session-id <ID> --sender 0xMyAddr --body "메시지"
xgram session reflect --session-id <ID>
xgram session recall --query "검색어" --k 5

# 5. L2 memory
xgram memory add --kind fact --content "물은 100도에 끓는다"
xgram memory list --kind fact
```

### 머신 간 메모리 이동 (PRD §20 F)

```bash
# 머신 A
xgram session export --session-id <ID> --out pkg.json

# 머신 B (init 완료 상태)
xgram session import --input pkg.json --verify   # ECDSA 서명 검증
```

### Cold backup 라운드트립

```bash
xgram uninstall --cold-backup-to ~/snap.tar.gz.enc
xgram restore --input ~/snap.tar.gz.enc --target-dir ~/.openxgram
xgram doctor   # 모든 layer 복원 확인
```

### Claude Code MCP 통합

```bash
# Claude Code 의 MCP 설정에 다음 추가:
{
  "mcpServers": {
    "openxgram": {
      "command": "xgram",
      "args": ["mcp-serve"],
      "env": { "XGRAM_KEYSTORE_PASSWORD": "..." }
    }
  }
}
```

세 tool 노출: `list_sessions`, `recall_messages`, `list_memories_by_kind`.

### 인터랙티브 마법사

```bash
xgram wizard   # ratatui state machine: Welcome → MachineId → Confirm
```

## 명령 매트릭스 (Phase 1)

설치 / 운영:
- `init` / `uninstall` / `reset` / `migrate` / `doctor` / `status`
- `daemon` / `daemon-install` / `daemon-uninstall`
- `restore` (cold backup)

데이터:
- `keypair new/list/show/import/export`
- `session new/list/show/message/reflect/recall/export/import/delete/reflect-all`
- `memory add/list/pin/unpin`

통합:
- `mcp-serve` — Claude Code MCP
- `notify discord/telegram` — webhook/bot 알림
- `wizard` / `tui` — 인터랙티브 화면

## Phase 1 MVP 진행률

- ✅ 9 crate 워크스페이스 (core / keystore / db / manifest / memory / transport / adapter / scheduler / mcp / cli)
- ✅ MVP 코어 명령 6/6 (init / uninstall / doctor / status / reset / migrate)
- ✅ L0 messages + L1 episodes + L2 memories + sqlite-vec KNN
- ✅ secp256k1 ECDSA 서명·검증 (메시지 / install-manifest)
- ✅ ChaCha20-Poly1305 keystore + cold backup + restore
- ✅ axum + reqwest localhost transport / `/v1/health`
- ✅ Discord webhook + Telegram bot
- ✅ tokio-cron-scheduler nightly reflection
- ✅ MCP JSON-RPC stdio 서버 (3 db tools)
- ✅ ratatui wizard state machine (3 화면)
- ✅ systemd user unit 생성기
- ✅ session export/import 라운드트립 + ECDSA 검증
- ✅ fastembed multilingual-e5-small (optional feature)

후속 (Phase 1.5+):
- restore 병합 모드, cold backup auto cron
- 9단계 wizard 추가 단계 (시드/패스워드/외부 어댑터 등)
- Tailscale 실 IP / mTLS
- HTTP MCP transport, fastembed 활성 시 의미 검색 통합
- L3 patterns / L4 traits 분류기
- Vault ACL 침투 테스트 자동화

## 빌드 환경 의존성

- Rust 1.75+ (async fn in trait stable)
- `apt install pkg-config libssl-dev` — `--features fastembed` 빌드 시
- `systemd --user` — `daemon-install` 사용 시 (Linux/macOS)

## 데이터 디렉토리

OpenXgram은 사용자 홈 디렉토리에 다음 구조로 데이터를 저장합니다:

```
~/.openxgram/
├── keystore/       # secp256k1 키페어 (권한 700)
├── data.db         # SQLite DB (메모리 레이어 + Vault)
└── config.toml     # 로컬 설정
```

## 기여 안내

버그 신고, 기능 제안, PR 모두 환영합니다. 자세한 내용은 [CONTRIBUTING.md](./CONTRIBUTING.md)를 참조하세요.

처음 기여하신다면 `good-first-issue` 라벨이 붙은 이슈를 찾아보세요.

## 라이선스

MIT License — [LICENSE](./LICENSE) 참조.

Copyright (c) 2026 OpenXgram Contributors
