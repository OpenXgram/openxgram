# OpenXgram

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/OpenXgram/openxgram/blob/main/LICENSE)
![CI](https://github.com/OpenXgram/openxgram/actions/workflows/ci.yml/badge.svg)
[![Version](https://img.shields.io/badge/version-0.2.0--rc.2-blue)](https://github.com/OpenXgram/openxgram/blob/main/version.json)
[![Status](https://img.shields.io/badge/status-production%20candidate-green)](https://github.com/OpenXgram/openxgram/blob/main/CHANGELOG.md)

**Repository**: https://github.com/OpenXgram/openxgram  
**Homepage**: https://openxgram.org

**OpenXgram — 자율 AI 에이전트 네트워크의 인프라**

> **에이전트끼리 메시지 · 결제 · 기억 · 신원을 공유하며 자가진화하는 경제. 사람은 Discord/Telegram으로 들여다본다.**

OpenXgram은 자율 AI 에이전트가 살고 일하는 기반 인프라다. 에이전트끼리 P2P로 통신(Nostr/XMTP)하고, USDC on Base로 가치를 교환하며, 5층 메모리(L0 messages → L4 traits)로 자가진화하고, secp256k1 HD 키페어 신원으로 어떤 LLM·머신에서도 같은 에이전트로 attach한다. 메시지는 에이전트 communication의 핵심 primitive이고, 메모리·Vault·신원은 에이전트가 자율로 움직이게 하는 기반. 사람은 Discord/Telegram을 창으로 이 네트워크를 관찰·개입한다. Claude · ChatGPT · Gemini · 로컬 모델 — LLM 무관, 데이터는 당신의 머신을 떠나지 않는다.

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

## 빠른 시작

### 설치

```bash
# 권장 — 사전 빌드 바이너리 자동 다운로드 (Linux x86_64/aarch64 · macOS x86_64/aarch64 · Windows x86_64)
curl -sSfL https://openxgram.org/install.sh | sh
xgram --version                             # openxgram 0.2.0
```

`install.sh` 가 OS·arch 를 자동 감지하여 [GitHub Releases](https://github.com/OpenXgram/openxgram/releases/latest) 에서 받습니다. Windows 는 `.zip` 직접 다운로드 또는 WSL2 권장.

### (선택) 소스에서 빌드 — 사전 빌드가 없는 플랫폼·고급 사용자

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
xgram memory list                  # 모든 종류 출력 (--kind 생략 가능)
xgram memory list --kind fact      # 특정 종류 필터
```

### 머신 간 메모리 이동 (PRD §20 F)

```bash
# 머신 A
xgram session export --session-id <ID> --out pkg.json

# 머신 B (init 완료 상태)
xgram session import --input pkg.json --verify   # ECDSA 서명 검증
```

### Cold backup

비파괴 백업 (수동 또는 systemd timer):

```bash
xgram backup --to ~/snap.cbk                          # 명시 파일 경로
xgram backup --to ~/.openxgram/backups                # 디렉토리 → timestamped
xgram backup-install --backup-dir ~/.openxgram/backups   # systemd .timer 자동화
systemctl --user enable --now openxgram-backup.timer
```

destructive 백업 + 복원:

```bash
xgram uninstall --cold-backup-to ~/snap.tar.gz.enc
xgram restore --input ~/snap.tar.gz.enc --target-dir ~/.openxgram
xgram doctor   # 모든 layer 복원 확인
```

### 자격증명 vault

```bash
xgram vault set --key discord/bot --value "TOKEN" --tags discord,prod
xgram vault list
xgram vault get --key discord/bot
xgram vault delete --key discord/bot
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

기본 tool 3종: `list_sessions`, `recall_messages`, `list_memories_by_kind`.
`XGRAM_KEYSTORE_PASSWORD` 환경 시 추가 노출: `vault_list`, `vault_get`, `vault_set`.

다른 클라이언트(비-Claude Code)는 HTTP transport 사용:

```bash
xgram mcp-serve --bind 127.0.0.1:7301
# POST http://127.0.0.1:7301/rpc — JSON-RPC 2.0 (initialize / tools/list / tools/call)
# GET  http://127.0.0.1:7301/health
```

### Tailscale 자동 bind (PRD §15)

WireGuard 터널이 네트워크 레이어에서 mTLS 제공 — axum-level TLS 불필요.

```bash
xgram daemon --tailscale            # `tailscale ip --4` 결과로 자동 bind
xgram doctor                        # Tailscale 상태(BackendState + IPv4) 검사
```

### Vault ACL · 일일 한도 · 감사 로그 · 정책

```bash
# 1. ACL 등록 — agent 가 실수로 vault 를 조작하지 못하도록
xgram vault acl-set \
    --key-pattern 'discord/*' --agent 0xAlice \
    --actions get,set --daily-limit 10 --policy auto

# 2. confirm 정책 — 마스터 승인 큐
xgram vault acl-set --key-pattern secret-key --agent 0xAlice \
    --actions get --policy confirm
xgram vault pending                  # 대기열 확인
xgram vault approve <id>             # 1회 승인 (consume)
xgram vault deny <id>

# 3. mfa 정책 — TOTP (RFC 6238, SHA1, 6자리, 30s)
xgram vault acl-set --key-pattern secret-key --agent 0xAlice \
    --actions get --policy mfa
xgram vault mfa-issue --agent 0xAlice  # base32 secret 발급 (Authenticator 등록)
```

### 인터랙티브 마법사

```bash
xgram wizard   # 9단계: Welcome → Alias → Role → DataDir → SeedMode → Adapter → Bind → Daemon → Backup → Confirm → Done
```

## 명령 매트릭스

설치 / 운영:
- `init` / `uninstall` / `reset` / `migrate` / `doctor` / `status`
- `daemon` (`--tailscale` 자동 bind) / `daemon-install` / `daemon-uninstall`
- `backup` (비파괴 cold backup) / `restore` (`--merge` non-empty 덮어쓰기)
- `backup-install` / `backup-uninstall` (systemd .timer 기반 주기 백업)

데이터:
- `keypair new/list/show/import/export`
- `session new/list/show/message/reflect/recall/export/import/delete/reflect-all`
- `memory add/list/pin/unpin`
- `patterns observe/list` (L3 — NEW/RECURRING/ROUTINE)
- `traits set/get/list/derive` (L4 — manual + L3 ROUTINE 자동 도출)
- `vault set/get/list/delete` (ChaCha20 암호화 자격증명)
- `vault acl-set/acl-list/acl-delete` (agent 권한 + 일일 한도 + 정책)
- `vault pending/approve/deny` (confirm 정책 승인 큐)
- `vault mfa-issue --agent <agent>` (TOTP secret 발급)

통합:
- `mcp-serve` (stdio) / `mcp-serve --bind <ADDR>` (HTTP transport)
- MCP tools: `list_sessions` · `recall_messages` · `list_memories_by_kind` · (vault: `vault_list` · `vault_get` · `vault_set`)
- `notify discord/telegram` — webhook/bot 알림
- `backup-push` — Discord/Telegram 으로 session 통계 push
- `wizard` (9단계) / `tui` — 인터랙티브 화면

## 현재 출하 표면 (v0.2.0-rc.2)

> Phase 1 GA(v0.1.0) + Phase 2 GA(v0.2.0) 누적. 세부 변경은 [CHANGELOG.md](CHANGELOG.md) 참조.

- ✅ 17 crate 워크스페이스 (core / keystore / db / manifest / memory / transport / adapter / scheduler / mcp / vault / cli / peer / payment / nostr / orchestration / channel / did)
- ✅ MVP 코어 명령 6/6 (init / uninstall / doctor / status / reset / migrate)
- ✅ 5층 메모리 CLI 표면: L0 messages / L1 episodes / L2 memories / L3 patterns / L4 traits
- ✅ L3 ROUTINE → L4 traits 자동 도출 (nightly reflection 통합 + 수동 트리거 `xgram traits derive`)
- ✅ sqlite-vec KNN + 런타임 임베더 선택 (`default_embedder()` — `--features fastembed` 빌드 시 multilingual-e5-small, 그 외 DummyEmbedder)
- ✅ secp256k1 ECDSA 서명·검증 (메시지 / install-manifest)
- ✅ ChaCha20-Poly1305 keystore + cold backup + restore (`--merge` non-empty 덮어쓰기)
- ✅ Vault — ChaCha20-Poly1305 자격증명 저장소
- ✅ Vault ACL — agent × key 패턴 매칭 + 일일 한도 + 감사 로그 (vault_audit)
- ✅ Vault confirm 정책 — pending 큐 + 마스터 승인 / 거부 / 1회 소비
- ✅ Vault mfa 정책 — RFC 6238 TOTP (SHA1, 6자리, 30s) + base32 secret 발급
- ✅ axum + reqwest localhost transport / `/v1/health`
- ✅ Discord webhook + Telegram bot
- ✅ tokio-cron-scheduler nightly reflection (reflect_all + derive_traits)
- ✅ MCP JSON-RPC stdio 서버 (db tools 3종 + vault tools 3종)
- ✅ MCP HTTP transport (`xgram mcp-serve --bind <ADDR>`)
- ✅ Tailscale 통합 — `xgram daemon --tailscale`, doctor 점검
- ✅ ratatui wizard 9단계 state machine (alias/role/data_dir/seed/adapter/bind/daemon/backup)
- ✅ systemd user unit 생성기 (sidecar daemon + backup .service/.timer 자동화)
- ✅ session export/import 라운드트립 + ECDSA 검증
- ✅ doctor — 9 체크 (manifest · data_dir · sqlite · keystore · drift · transport · memory · vault · embedder · tailscale)

Phase 2 추가 (v0.2.0):
- ✅ Nostr P2P transport — application-layer ratchet, 자체 호스팅 relay, `nostr://` peer scheme
- ✅ USDC on Base 실 송금 (alloy + LocalSigner + Base Sepolia 통합 테스트)
- ✅ Audit chain — append-only Merkle hash chain + CLI 노출
- ✅ KEK rotation — 키 회전 + envelope re-wrap + grace window
- ✅ MFA — WebAuthn + TOTP fallback
- ✅ OpenTelemetry — traces/metrics + OTLP exporter
- ✅ Retention — L0~L4 보존 정책 + TTL 자동 정리
- ✅ Backup — age 기반 비파괴 백업 + restore 라운드트립
- ✅ Tauri GUI — 데스크톱 베이스라인 + IPC + 보안 CSP
- ✅ openxgram.org 정적 사이트 + GitHub Pages

향후 (Phase 3 후보 — 미정):
- Mobile (iOS/Android) sidecar
- 추가 결제 네트워크 / Stablecoin
- Federation: 머신 메쉬 자동 토폴로지

## 프로덕션 후보 (Production Candidate)

v0.2.0-rc.1은 Phase 1 + Phase 2 누적 모든 기능을 갖춘 **release candidate**. 2~4주 외부 검증 기간을 거쳐 v0.2.0 stable로 graduate 예정. 이 기간 동안:

- 다중 OS·머신 환경 실사용 검증 환영
- 버그·엣지케이스 [GitHub Issues](https://github.com/OpenXgram/openxgram/issues)로 신고
- 보안 이슈는 [SECURITY.md](SECURITY.md) 절차에 따라 비공개 보고
- 마이너 API 변경 가능 (rc 단계의 일반적 권한)

> ⚠️ **자격증명·비밀 인프라**: vault에 운영 환경 비밀을 저장하기 전, 백업/복원 라운드트립을 먼저 검증하세요. `xgram backup` + `xgram restore` 시나리오 통과 후 본격 사용 권장.

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
