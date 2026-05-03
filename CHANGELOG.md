# Changelog

OpenXgram 의 변경 이력. 모든 시간은 KST(Asia/Seoul). [Semantic Versioning](https://semver.org/) + BUILD 자동 증가 (CI/CD 갱신, 수동 변경 금지).

## [Unreleased] — Phase 1 MVP

### Added
- **11 crate 워크스페이스** — core / keystore / db / manifest / memory / transport / adapter / scheduler / mcp / vault / cli
- **CLI 코어 명령 6/6** — `init` `uninstall` `doctor` `status` `reset` `migrate`
- **session 명령** — `new` `list` `show` `message`(서명) `reflect` `reflect-all` `recall` `export` `import`(`--verify`) `delete`
- **memory 명령** — `add` `list` `pin` `unpin` (L2 fact/decision/reference/rule)
- **patterns 명령** — `observe` `list` (L3 NEW/RECURRING/ROUTINE 빈도 기반 분류)
- **traits 명령** — `set` `get` `list` (L4 정체성·성향, manual source)
- **vault 명령** — `set` `get` `list` `delete` (ChaCha20-Poly1305 자격증명 저장소)
- **keypair 명령** — `new` `list` `show` `import` `export` (BIP39 + HD m/44'/60'/0'/0/0)
- **운영 명령** — `daemon` `daemon-install` `daemon-uninstall` `backup` `backup-install` `backup-uninstall` `restore` `mcp-serve` `notify` `backup-push` `tui` `wizard`
- **L0~L4 메모리 레이어 CLI 노출** — messages + episodes + memories + patterns + traits + sqlite-vec KNN (384d, multilingual-e5-small 호환)
- **install-manifest 인프라** — SPEC §4.1 13 필수 필드 + atomic IO + secp256k1 서명·검증 + drift 감지
- **암호화** — ChaCha20-Poly1305 + Argon2id (keystore V3 + cold backup blob)
- **Memory Transfer (PRD §17)** — text-package-v1 export/import 머신 간 라운드트립 + master_public_key 동봉 + ECDSA verify
- **Cold backup** — tar.gz + ChaCha20 → restore 라운드트립
- **Transport baseline** — axum HTTP `POST /v1/message` + `GET /v1/health`
- **Adapter** — Discord webhook + Telegram bot (wire-level wiremock 검증)
- **Scheduler** — tokio-cron-scheduler 야간 reflection job (`0 0 15 * * *` UTC = 자정 KST)
- **MCP 서버** — JSON-RPC stdio + db tools 3종 (`list_sessions`/`recall_messages`/`list_memories_by_kind`) + vault tools 3종 (`vault_list`/`vault_get`/`vault_set`, `XGRAM_KEYSTORE_PASSWORD` 환경 시 노출)
- **TUI** — ratatui welcome 화면 + 9단계 wizard state machine baseline (Welcome → MachineId → Confirm → Done, Esc/B 이전단계)
- **systemd user units** — sidecar daemon `.service` + cold backup `.service` + `.timer` 자동화 (기본 OnCalendar=Sun 03:00:00, Persistent=true)
- **비파괴 cold backup** — `xgram backup` 명령 + 디렉토리 입력 시 KST timestamped 파일명 자동 생성
- **fastembed optional feature** — multilingual-e5-small ONNX 모델 (`--features fastembed`)

### Quality
- **clippy** — `--workspace --all-targets -- -D warnings` 0 errors / 0 warnings
- **통합 테스트** — 100+ 건 전부 통과 (db / keystore / manifest / memory recall+reflection+memories / transport / adapter / cli init+uninstall+doctor+status_reset+session+memory+notify+migrate+daemon+wizard+systemd+mcp_serve+cold_backup+tui)
- **Silent error 4패턴** 전 crate 적용 — reqwest `.error_for_status()?` / rusqlite `affected_rows()` / tokio-cron-scheduler panic 핸들러 / keyring round-trip
- **마스터 코드 작성 규칙** 준수 — 응집도 분리, 중복 금지(core hub), 중앙화(paths/time/env/confirm/ports), 하드코딩 제거, 모듈화 4원칙

### Phase 1.5+ 잔여
- 9 단계 wizard 추가 단계 (시드 / 패스워드 / 외부 어댑터 / Transport / 데몬 등록)
- Tailscale 실 IP / mTLS transport
- HTTP MCP transport
- fastembed 활성 시 MessageStore embedder 통합 (현재 DummyEmbedder)
- L3 → L4 traits 자동 도출 (야간 reflection)
- Vault ACL · 일일 한도 · MFA 정책 + 침투 테스트
- restore 병합 모드

---

## 0.1.0-alpha.1 — 2026-04-30 KST (bootstrap)

- 초기 워크스페이스 + PRD/SPEC/체크리스트
