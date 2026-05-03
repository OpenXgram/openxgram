# Changelog

OpenXgram 의 변경 이력. 모든 시간은 KST(Asia/Seoul). [Semantic Versioning](https://semver.org/) + BUILD 자동 증가 (CI/CD 갱신, 수동 변경 금지).

## [0.1.0-rc.2] — 2026-05-04 KST (Phase 1 RC2 — vault notify + MCP HTTP auth)

### Added
- **Vault pending Discord 알림** — `DISCORD_WEBHOOK_URL` 환경 시 confirm 정책으로 pending 생성될 때 마스터에게 fire-and-forget POST. silent error 패턴, `XGRAM_VAULT_NOTIFY=off` 로 비활성화.
- **MCP HTTP caller 인증** — Bearer 토큰 발급/검증/폐기 (`xgram mcp token-create/list/revoke`). DB 에 SHA-256 해시만 저장. agent 식별 후 vault tools (`vault_get`/`vault_set`/`vault_delete`) 가 ACL · 일일한도 · confirm/mfa 정책에 자동 라우팅. `XGRAM_MCP_REQUIRE_AUTH=1` 으로 헤더 없는 요청 reject (엄격 모드). 헤더 없으면 master 폴백.
- **MCP MFA 코드 동봉** — vault_get/_set arg 의 `mfa_code` 필드 (TOTP 코드).

## [0.1.0-rc.1] — 2026-05-04 KST (Phase 1 MVP RC)

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
- **MCP 서버** — JSON-RPC stdio + HTTP (`mcp-serve --bind`) + db tools 3종 (`list_sessions`/`recall_messages`/`list_memories_by_kind`) + vault tools 3종 (`vault_list`/`vault_get`/`vault_set`, `XGRAM_KEYSTORE_PASSWORD` 환경 시 노출)
- **TUI** — ratatui 9단계 wizard (alias/role/data_dir/seed/adapter/bind/daemon/backup → confirm → done, Esc/B 이전단계, cfg 보존)
- **systemd user units** — sidecar daemon `.service` + cold backup `.service` + `.timer` 자동화 (기본 OnCalendar=Sun 03:00:00, Persistent=true)
- **비파괴 cold backup** — `xgram backup` 명령 + 디렉토리 입력 시 KST timestamped 파일명 자동 생성
- **restore --merge** — 비어있지 않은 target_dir 에 백업 덮어쓰기 (target only 파일 보존)
- **Vault ACL · 감사 로그 · 일일 한도** — agent × key 패턴 매칭, vault_audit 전수 기록, 정책 (auto/confirm/mfa)
- **Vault confirm 정책** — pending 큐 + 마스터 승인 (`vault pending`/`approve`/`deny`) + 1회 소비
- **Vault mfa 정책** — RFC 6238 TOTP (SHA1/6자리/30s) + base32 secret 발급 (`vault mfa-issue`)
- **L3 → L4 traits 자동 도출** — ROUTINE pattern 을 derived trait 로 upsert (nightly reflection 통합 + 수동 `traits derive`)
- **Tailscale 통합** — `xgram daemon --tailscale` 자동 bind, doctor BackendState 검사 (mTLS = WireGuard 레이어)
- **default_embedder() factory** — `--features fastembed` + `XGRAM_EMBEDDER!=dummy` → FastEmbedder, 그 외 → DummyEmbedder. MessageStore 가 `?Sized` 로 Box<dyn Embedder> 수용
- **doctor 9 체크** — manifest / data_dir / sqlite / keystore / drift / transport / memory / vault / embedder / tailscale
- **fastembed optional feature** — multilingual-e5-small ONNX 모델 (`--features fastembed`)

### Quality
- **clippy** — `--workspace --all-targets -- -D warnings` 0 errors / 0 warnings
- **통합 테스트** — 100+ 건 전부 통과 (db / keystore / manifest / memory recall+reflection+memories / transport / adapter / cli init+uninstall+doctor+status_reset+session+memory+notify+migrate+daemon+wizard+systemd+mcp_serve+cold_backup+tui)
- **Silent error 4패턴** 전 crate 적용 — reqwest `.error_for_status()?` / rusqlite `affected_rows()` / tokio-cron-scheduler panic 핸들러 / keyring round-trip
- **마스터 코드 작성 규칙** 준수 — 응집도 분리, 중복 금지(core hub), 중앙화(paths/time/env/confirm/ports), 하드코딩 제거, 모듈화 4원칙

### Phase 2+ 후속
- 통합 테스트 격리 강화 (serial_test 또는 동적 포트 → CI 병렬화)
- MCP HTTP caller 인증 (현재 master-context 가정 — agent 식별용 token/header)
- daemon 측 vault pending Discord/Telegram 알림 (#57 후속)
- Tauri GUI · XMTP 어댑터 · USDC 결제 (PRD §16)

---

## 0.1.0-alpha.1 — 2026-04-30 KST (bootstrap)

- 초기 워크스페이스 + PRD/SPEC/체크리스트
