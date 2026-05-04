# OpenXgram 사용자 가이드 — Use Cases

OpenXgram 의 핵심 사용 시나리오 8 개. 각 문서는 실제 작동하는 `xgram` CLI 명령으로 구성된다. 후속 PR 에서 노출 예정인 명령은 본문에 명시한다.

## 인덱스

- [01 — 여러 머신에서 동일 세션 이어가기](./01-cross-machine-session.md) — 시드 import + `xgram session export/import --verify` 로 세션을 다른 머신으로 이전.
- [02 — Vault 로 API 키 안전 공유](./02-vault-credential-share.md) — `xgram vault set/get/list/acl-set` 로 ChaCha20 암호화 + ACL · 일일 한도 · 정책(auto/confirm/mfa) 부여.
- [03 — USDC 자동 결제 + MFA](./03-usdc-payment-automation.md) — `xgram payment new/sign/mark-submitted/mark-confirmed` intent 라이프사이클 (chain=base, USDC).
- [04 — Nostr cross-network 메시지](./04-nostr-cross-network.md) — `nostr://` 주소 peer 등록 + `xgram peer send/broadcast` 로 NAT/방화벽 너머 통신.
- [05 — 5층 메모리로 에이전트 학습 관리](./05-memory-layers.md) — L0~L4 (`session message/reflect`, `memory add/pin`, `patterns observe`, `traits set`, KNN `recall`).
- [06 — KEK 회전 (rotate-kek + audit chain)](./06-vault-rotate-kek.md) — KEK 회전 + audit_chain 무결성 검증 (`xgram audit verify` / `xgram vault rotate-kek` 는 후속 PR — 라이브러리 노출 완료).
- [07 — Retention 정책 적용 (90 일 L0 압축)](./07-retention-cleanup.md) — `xgram retention run/compact` 로 오래된 messages 압축 (라이브러리 완성, top-level 명령 노출은 후속 PR).
- [08 — age multi-recipient backup + 복구](./08-backup-restore.md) — `xgram backup` / `xgram restore --merge` / `xgram backup-install` / `xgram backup-push` (age multi-recipient 는 후속 PR).

## 공통 사전 준비

- `xgram init --alias <머신별칭> --role primary` 로 머신 초기화
- `XGRAM_KEYSTORE_PASSWORD` 환경변수 export — keystore unwrap 에 사용
- 데몬 백그라운드 가동: `xgram daemon-install --bind 127.0.0.1:7300` → `systemctl --user enable --now openxgram-sidecar.service`

## 절대 규칙 (모든 케이스에 동일 적용)

- fallback 금지 — 모든 오류는 raise. silent degrade 절대 없음.
- 롤백 가능 후 자동 — destructive 동작은 사전 backup + 명시 confirm.
- DB 변경은 마스터 승인 — INSERT/UPDATE/DELETE 는 마스터 명시 호출만.
- KST 시간대 — 모든 timestamp Asia/Seoul, UTC 사용 금지.
- 표(table) 사용 금지 — 보고·문서 모두 목록.

## CLI 명령 빠른 참조 (Phase 1 노출)

- 핵심: `init` `status` `doctor` `reset` `migrate` `uninstall` `version` `completions` `dump`
- Session/Memory: `session new|list|show|message|reflect|reflect-all|recall|export|import|delete` / `memory add|list|pin|unpin` / `patterns observe|list` / `traits set|get|list`
- 자격증명: `keypair new|list|show|import|export` / `vault set|get|list|delete|acl-set|acl-list|acl-delete|pending|approve|deny|mfa-issue`
- 통신: `peer add|list|show|touch|delete|send|broadcast` (http:// / nostr:// scheme 자동 라우팅) / `relay` (자체 호스팅 Nostr relay)
- 운영: `daemon` `daemon-install` `daemon-uninstall` `backup` `backup-install` `backup-uninstall` `restore` `mcp-serve` `mcp-token` `notify` `backup-push` `tui` `wizard`
- 결제: `payment new|sign|list|show|chains|mark-submitted|mark-confirmed|mark-failed`

## 후속 PR 노출 예정 (라이브러리 단계)

- `xgram audit verify|backfill|checkpoint` — `crates/openxgram-cli/src/audit.rs` 구현 완료, top-level 미노출.
- `xgram vault rotate-kek` — KEK 회전 통합테스트로 검증, CLI 노출은 후속 PR.
- `xgram retention run|compact` — `crates/openxgram-cli/src/retention.rs` 11 KB 구현, top-level 미노출.
- `xgram nostr-inbound run` / `xgram ratchet-cron run` — sidecar 통합 형태로만 동작, 단독 명령 미노출.
- age multi-recipient backup — `crates/openxgram-cli/src/age_backup.rs` 구현 부분 (`encrypt_with_passphrase`), 다중 X25519 수신자 CLI 미노출.
