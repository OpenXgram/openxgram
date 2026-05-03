# OpenXgram

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

## 시작하는 법

Phase 1 MVP 구현 중. 추후 업데이트 예정.

```bash
# 예정 (구현 완료 후)
cargo install openxgram
xgram daemon --start
xgram attach --agent akashic
```

## 라이선스

미정 (Phase 1 완료 후 결정)
