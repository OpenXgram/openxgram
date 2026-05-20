# Changelog

OpenXgram 의 변경 이력. 모든 시간은 KST(Asia/Seoul). [Semantic Versioning](https://semver.org/) + BUILD 자동 증가 (CI/CD 갱신, 수동 변경 금지).

## [0.2.0-rc.30] — 2026-05-20 KST (UI-MESSENGER-SPEC v1.3 완전 구현)

사양 43+16=59 결정 모두 코드 골격 + 실 enforcement workers 5종 가동.

**백엔드 신규 API (15+ endpoint)**:
- 머신×세션 detector: `/v1/gui/sessions` (tmux + Claude projects + ps 통합), `/v1/gui/machine`, `/v1/gui/sessions/{id}/screen` (xterm.js stream)
- 마스터+서브 지갑 (M-3 + L4 영구 점유 + M-6 + S6 + V8): `/v1/gui/wallets`, `/wallets`, `/wallets/topup`
- 승인 큐 (L6 + V4): `/v1/gui/approvals`
- 정책: `/v1/gui/role-policies` (L3 + V1), `/v1/gui/whitelist` + `/whitelist-patterns` (M-5 + N1 + N3 + V4)
- 글로벌 검색 (N4): `/v1/gui/search` (FTS5)
- 라우팅 규칙 (V11): `/v1/gui/routing-rules`
- 버전 (V12): `/v1/gui/version`
- 시스템 cron 보호 (N7): `/v1/gui/system-cron/protect-attempt`
- 첨부 (S7 + V2/V3): `/v1/gui/attachments` + GET/POST + refcount + content-addressed
- cross-machine queue (S8 + V6): `/v1/gui/cross-machine-queue`

**프론트 UI**:
- 메신저 좌측 (S4 collapse + 정렬·필터): 머신×세션 트리, tmux + Claude project 실시간
- 메신저 중앙 (S5): xterm.js + 2초 폴링 (tmux capture-pane / .jsonl tail)
- 메신저 우측 12 탭 (S3): 개요·역할·채널바인딩·상태·히스토리·내보내기·지갑·토큰·Cron·파일·알림·권한·도구·MCP
- 메신저 헤더 🔀: RoutingRulesModal (V11)
- 글로벌 헤더 🔍: GlobalSearchModal (N4), 🔔: ApprovalQueueBell (L6)
- C5 breadcrumb 7 카드 전부 적용 (Memory, Identity, Vault, Channel, Autonomy, External, Ops)

**Daemon enforcement workers (5종 가동)**:
- M-4 (60s): agent_identities 휴면 자동 전이 (Active/Idle/Dormant/Offline) + N8 lifecycle_log
- M-5 (60s): 화이트리스트 매칭 시 auto_register + whitelist_match_log
- M-6 (60s): sub_wallets auto_topup_enabled 자동 충전 (일 한도 내)
- L6 (30s): vault_pending 24h 경과 Expired
- V6 (10s): outbound_queue 30일 archive + 10회 실패 dead-letter

**마이그레이션 신규 3종**:
- v21 sub_wallets: M-3 + L4 + M-6 (HD 영구 점유 + 자동 충전 schema)
- v23 messenger_full: M-2 agent_identities, V6 outbound_queue, N4 global_search FTS5, V11 routing_rules, V12 version_log, N7 protect_log, N8 lifecycle_log
- v24 messenger_attachments: S7 attachment_refs + inline + M-5 whitelist_patterns + match_log

**잔여 (사양 명시)**: S6 LLM 실 토큰비 정확 (현재 message length proxy), S7 disk 저장 (현재 inline only), S8 transport 측 sender 통합, N4 sqlite-vec 시멘틱 (FTS5 만).

## [0.2.0-rc.29] — 2026-05-20 KST (HomeDashboard 8 카드 + 7 카드 컴포넌트 + UI 버전 표시)

**누적 publish** — rc.23~rc.28 은 로컬 태그였고 GitHub Releases 에 publish 안 됨. rc.29 가 rc.22 (5월 11일) 이후 첫 공식 release. 묶인 변경:

- **rc.24 (Tauri → 웹 GUI / Tailscale Funnel)** — `xgram-desktop` 폐기, `xgram gui` = Funnel URL → 브라우저. release-binaries Tauri step 제거 → 빌드 시간 30분 → 5~10분.
- **rc.25 (단일 사용자 잠금 — PRD §1)** — `register`/`login`/`me`/`logout` (JWT) → `unlock`/`check` (keystore 비밀번호 + session_token).
- **rc.26 (daemon GUI embed + Discord listener)** — `include_dir` 매크로로 `ui/web/dist` 를 xgram 바이너리에 임베드. Discord listener daemon 통합 + GUI 페어링 카드.
- **rc.27 (Messenger 4 Tier + Step 0)** — 메시지 송수신 API + 좌측 머신×세션 트리 + 4-tuple + 에이전트/스레드 2-모드 + 우측 12탭 MVP 5탭 + 사용자 개입 토글 + Hand-off radio.
- **rc.28 (테마 일관성)** — prefers-color-scheme 다크/라이트 자동 전환, inline rgba 색 → CSS 변수.
- **rc.29 (HomeDashboard + 카드 컴포넌트 + 버전 표시)** — unlock 후 첫 화면 = 8 카드 (4 가치 + 4 토대). 7 카드 컴포넌트 신규: Identity·VaultMcp·Channel·Memory·Autonomy (사양 §3 기반) + ExternalAgent·Ops (책임 placeholder). `ui/web/vite.config.ts` define 으로 `__APP_VERSION__`·`__BUILD_TIME__` 빌드 시 주입 → App header 에 `v0.2.0-rc.29` 표시 (CLAUDE.md 룰 6 준수).
- **install.sh fixes** — Tailscale Funnel 자동 활성화, sudo prompt /dev/tty, python3 의존 제거, hostname 3단계 fallback, Funnel target 47302 (daemon listen port).

## [0.2.0-rc.25] — 2026-05-19 KST (단일 사용자 잠금 — PRD §1 정렬)

**컨셉 정정**: PRD §1 = **1 사람 = 1 메인 daemon + N 머신 attach**. 이전 rc.24 에서 잘못 추가한 multi-user/register/users 테이블/JWT 흐름을 폐기.

- **auth 모델 단순화** — `register`/`login`/`me`/`logout` (JWT) → `unlock`/`check` (keystore 비밀번호 + session_token). `XGRAM_KEYSTORE_PASSWORD` 환경변수 1개와 SHA256 비교, 일치 시 프로세스-수명 토큰 발급.
- **`crates/openxgram-cli/src/auth.rs`** 재작성 — `UnlockRequest`/`UnlockResponse`, `verify_password`, `session_token` (OnceLock), `verify_session_token`. argon2/jsonwebtoken 의존 호출 제거.
- **`crates/openxgram-cli/src/daemon_gui.rs`** — 라우터 `/v1/auth/{register,login,me,logout}` 4개 → `/v1/auth/{unlock,check}` 2개. `require_auth` = session_token 우선 → mcp-token fallback.
- **`crates/openxgram-db/src/migrate.rs`** — v22 `users` 마이그레이션 등록 해제 (적용된 DB는 그대로, 신규 설치는 생성 안 함).
- **`ui/web/src/components/LoginView.tsx`** — 이메일 필드 제거, 비밀번호 단일 필드. `RegisterView.tsx` **삭제**.
- **`ui/web/src/api/auth.ts`** — `unlock(password)` / `isUnlocked()` / `lock()`. localStorage `xgram_session_token`. register/login/logout API 호출 폐기.
- **`ui/web/src/App.tsx`** — `authScreen` signal·`RegisterView` import 제거. `isAuthenticated` → `isUnlocked`. `apiLogout()` → `lock()`.
- **검증** (e2e on `https://whitegun-win-1.tail0957ca.ts.net/`):
  - `POST /api/auth/unlock` (wrong pw) → 401
  - `POST /api/auth/unlock` (correct pw) → 200 + `session_token`
  - `GET /api/auth/check` (no token) → 401
  - `GET /api/auth/check` (Bearer) → 200
- **PRD-OpenXgram v1.2 → v1.3** — §9 결정 12 신설(단일 사용자 잠금 + multi-machine attach). §4.8 v0.9 인증 흐름 갱신.

## [0.2.0-rc.24] — 2026-05-19 KST (Tauri 폐기 → 웹 GUI / Tailscale Funnel)

**브레이킹**: Tauri 데스크톱 앱(`xgram-desktop`) 완전 폐기. 웹 GUI(Tailscale Funnel) 로 대체.

- **`xgram gui` 동작 변경** — 별 바이너리(`xgram-desktop`) 호출 폐지. `tailscale status --json` 으로 Funnel URL 추출 → OS 기본 브라우저(`start`/`open`/`xdg-open`) 로 오픈. 옵션 `--port <PORT>` (default 47310, nginx GUI 서빙 포트), `--no-open` (URL stdout 만).
- **release-binaries.yml 단순화** — Tauri 빌드 step·Node/Vite/libwebkit2gtk 설치 제거. 5 OS × 1 binary (xgram only). 빌드 시간 ~30분 → ~5~10분.
- **install.ps1 / install.sh** — `xgram-desktop` 다운로드·검증·복원 로직 제거. install.sh 는 옛 `xgram-desktop` 바이너리 자동 정리. 다음 단계 안내에 Tailscale Funnel 한 줄 추가.
- **`ui/tauri/DEPRECATED.md`** 신규 — 폐기 이유, 마이그레이션, `ui/web/` 가리킴. 코드는 보존(과거 release 사용자 참조용).
- **README §빠른 시작** — 3단계 흐름(install → init → tailscale funnel + xgram gui) 추가. "Tauri GUI" 표면 항목은 취소선 + 웹 GUI 전환 명시.
- **PRD-OpenXgram §4.8** — "Beta Tauri GUI" → "Beta 웹 GUI (Tailscale Funnel)". §9 결정 11 신설 (GUI 호스팅 = Tailscale Funnel default, Cloudflare 옵션). Phase v0.9 작업 명세 갱신. PRD 버전 v1.1 → v1.2.
- **빌드 의존성**: `webbrowser` crate 추가 없음 — `std::process::Command` 로 `start`/`open`/`xdg-open` 직접 호출. 코어 dep tree 변화 0.
- **Cargo.toml workspace version** rc.23 → rc.24. `exclude = ["ui/tauri"]` 유지.

## [0.2.0-rc.22] — 2026-05-11 KST (i18n + install 안내)

- **i18n 13개 누락 키 채움** + install.ps1 next-step 안내 확장.
- **fix(mcp/connect_discord)**: bot_token primary (양방향), webhook은 fallback only.

## [0.2.0-rc.21] — 2026-05-11 KST (자율 채널 연결)

- **bot register one-liner** + MCP self-install + identity inject + 글로벌 token auto-import.
- **MCP 액션 도구 3개** — `create_project_category` + `install_hooks` + `register_subagent`.
- **MCP `connect_discord` / `connect_telegram`** — LLM이 자연어로 채널 연결 실행.
- **`send_to_discord` / `send_to_telegram`** — LLM 자연어 응답 outbound 절반.

## [0.2.0-rc.16 ~ rc.20] — 2026-05-10 KST (Windows + GUI Messenger)

- **rc.16**: `core/paths` Windows USERPROFILE fallback — `xgram init` HOME 미설정 에러 해결 (#177).
- **rc.17**: GUI Messenger v0.2-α — L0 messages 활동 흐름 모니터링.
- **rc.18**: GUI Messenger v0.2-β — peer 송신 활성화.
- **rc.19**: tauri custom-protocol feature 활성화 — release에서 bundled frontend 로딩.
- **rc.20**: `dirs_home` USERPROFILE fallback — Windows 초기화 인식.
- **Windows install.ps1** — `irm | iex` 한 줄 설치 + ZipFile.ExtractToDirectory 호환 fix + 재설치 시 daemon+agent 자동 가동(idempotent) + 실행 중인 .exe 자동 종료.
- **Cloudflare _headers** — `install.ps1/install.sh` charset=utf-8 (nginx origin 환경에서 revert).

## [0.2.0-rc.13 ~ rc.15] — 2026-05-09 KST (Phase 1 Agent + EAS + Indexer + Identity Directory)

- **rc.13**: Telegram 양방향 (Phase 1 1.6) (#171).
- **rc.14**: chat REPL + bot mgmt + invite QR + HITL + EAS + indexer + import-app + multi-channel + identity directory (#172).
- **rc.15**: `xgram chat` 제거 반영 (#173) — 채팅 인터페이스는 외부 LLM/MCP로.

## [0.2.0-rc.8 ~ rc.12] — 2026-05-08~09 KST (Phase 1 Agent 본격 가동)

- **rc.8**: `xgram agent` — inbox 폴링 + Discord forward + install.sh 자동 가동 (Phase 1 v1) (#166).
- **rc.9**: Discord 양방향 통합 + `/v1/agent/inject` 엔드포인트 (Phase 1 1.5) (#167).
- **rc.10**: echo 응답 + outbox 메모리 (Phase 1 1.7 v0) (#168).
- **rc.11**: Anthropic LLM 응답 (Phase 1 1.7.3) (#169).
- **rc.12**: 서브에이전트 라우팅 v0 — single-LLM 멀티 페르소나 (Phase 1 1.8) (#170).

## [0.2.0-rc.4 ~ rc.7] — 2026-05-08 KST (Daemon GUI HTTP + Phase 2c)

- **rc.4**: daemon `/v1/gui/*` HTTP API + Tauri 원격 클라이언트 (#154).
- **rc.5**: `pair-desktop` / `link` 한 줄 페어링 (#156) — `oxg://alias@host:port#token=xxx` URL.
- **rc.6**: Phase 2c (schedule + chain + mutations) (#162).
- **rc.7**: axum 0.8 path syntax (`:id → {id}`) fix (#164).

## [0.2.0-rc.3] — 2026-05-08 KST (CI 그린화 + 신규 crate 3종)

- CI track `ui/tauri/app/package-lock.json` — release-binaries.yml 그린화 (#146).
- 신규 crate 추가: **openxgram-eas** (Ethereum Attestation Service).
- 신규 crate 추가: **openxgram-indexer-sdk** (이벤트 인덱서 베이스).
- 신규 crate 추가: **openxgram-wiki** — L2 Karpathy 위키 페이지 (PRD §4.1).

## [0.2.0-rc.2] — 2026-05-07 KST (UX 개선)

자체 dogfood에서 발견된 마찰점 1차 정리 — "심플·편리·단순" 원칙. 깊은 리팩터(포트 자동 폴백, `--data-dir` 글로벌화)는 rc.3로 분할.

- **`xgram setup discord` / `xgram setup telegram`** — 외부 채널 연결 단일 진입점 신규. 기존 `xgram notify setup-discord` / `setup-telegram` 도 호환 유지.
- **`xgram init` 플래그 없이 실행** — 인터랙티브 마법사(`xgram wizard`)로 자동 진입. `--alias`를 넘기면 비대화 모드 (기존 동작).
- **`install.sh` 사이트 배포 갭 수정** — `www/install.sh` → `www/public/install.sh` 이동. Vite의 publicDir만 dist로 복사되는 문제로 `https://openxgram.org/install.sh`가 404였던 것 정리.
- **`install.sh` 메시지 정정** — 끝의 "12-단어 복구 시드" → "BIP39 24단어" + `--alias` 안내 추가.

기능 변경 0건 (안정 표면 유지). UX 진입점 + 출하 파이프라인만 손봄.

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
