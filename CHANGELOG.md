# Changelog

OpenXgram 의 변경 이력. 모든 시간은 KST(Asia/Seoul). [Semantic Versioning](https://semver.org/) + BUILD 자동 증가 (CI/CD 갱신, 수동 변경 금지).

## [0.2.0-rc.1] — 2026-05-06 KST (Production Candidate)

Phase 2 GA 표면 동결 + 프로덕션 후보 진입. 외부 검증 기간(2~4주) 후 v0.2.0 stable로 graduate 예정.

- **Security CI** — `cargo audit` (RustSec advisories) + `cargo deny` (license / source / advisories) 워크플로우 추가. `deny.toml` 정책으로 MIT 호환 permissive 라이선스만 허용.
- **Transport** — rate-limit 테스트의 프로세스 env var race 구조적 해결: `spawn_server_with_rate_limit(addr, per_min)` export로 set_var/remove_var 의존 제거.
- **Doc** — phase-1-mvp.md / phase-2-roadmap.md에 ✅ COMPLETED 배너 추가, README "Phase 1" 라벨 정리, 출하 표면 17 crate 반영, "Production Candidate" 섹션 추가.
- **Workspace fmt drift** — main에 누적된 rustfmt 부채 29 파일 정리.
- **CI** — release-binaries에 macOS x86_64 cross-compile 추가 (macos-14 → x86_64-apple-darwin).

기능 변경 없음 — Phase 2 GA(v0.2.0) 표면 그대로. 외부 검증 가능 안정 표면 동결.

## [0.2.0] — 2026-05-04 KST (Phase 2 GA)

Phase 2 누적 — Nostr 기반 P2P 전송 / USDC 결제 / 신뢰 인프라 / 관측성 / 데스크톱 GUI / 공식 사이트.

- **Nostr (PRD-NOSTR-01~07)** — Keys conversion + kind 매핑, NostrSink/NostrSource, application-layer ratchet, 자체 호스팅 relay, `nostr://` peer scheme 라우팅.
- **Payment (PRD-PAY-01~08)** — USDC on Base alloy 베이스라인 + signer + RPC + testnet 통합 테스트 (Base Sepolia).
- **Audit chain (PRD-AUDIT-01~03)** — append-only Merkle hash chain + CLI 노출 + 검증.
- **KEK Rotation (PRD-ROT-01~02)** — 키 회전 + envelope re-wrap + grace window.
- **MFA (PRD-MFA-01)** — WebAuthn ADR + TOTP fallback 통합.
- **Observability (PRD-OTEL-01~03)** — OpenTelemetry traces/metrics + OTLP exporter + tracing-opentelemetry brigde.
- **Retention (PRD-RET-01~02)** — L0~L4 보존 정책 + TTL 자동 정리.
- **Backup (PRD-BAK-01~02)** — age 기반 비파괴 백업 + restore 라운드트립.
- **Tauri GUI (PRD-UI-01~03)** — 데스크톱 베이스라인 + IPC + 보안 CSP.
- **Site (PRD-SITE-01)** — openxgram.org 정적 사이트 + GitHub Pages 워크플로우.

세부 변경은 `docs/checklists/` Phase 2 체크리스트 참조.

## [0.1.0] — 2026-05-04 KST (Phase 1 GA)

rc.1 + rc.2 누적 — 기억·자격 인프라 첫 정식 릴리스. 외부 사용 가능 안정 표면.
세부 변경은 아래 [0.1.0-rc.2] / [0.1.0-rc.1] 섹션 참조.

핵심 표면:
- 5층 메모리 (L0 messages → L1 episodes → L2 memories → L3 patterns → L4 traits)
- L3 → L4 자동 도출 (nightly reflection 통합)
- Vault 4단계 보안 (ACL × 일일한도 × 감사로그 × auto/confirm/mfa 정책)
- MCP transport 2종 (stdio + HTTP Bearer 인증)
- Tailscale 자동 bind (mTLS = WireGuard 레이어)
- 비파괴 backup + systemd .timer 자동화 + restore --merge
- 9단계 interactive wizard
- doctor 10 체크 + JSON 출력
- 런타임 임베더 선택 (fastembed/dummy)
- 쉘 자동 완성 (bash/zsh/fish/elvish/powershell)
- 구조화 JSON 로그 (XGRAM_LOG_FORMAT=json)

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
