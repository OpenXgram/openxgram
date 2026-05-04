# 06 — KEK 회전 (rotate-kek + audit chain)

> 한 줄 요약: vault 의 KEK(Key-Encryption-Key)를 주기적으로 회전하고 audit chain 으로 무결성을 증명한다.

## 시나리오

마스터는 보안 정책상 분기마다 vault 의 KEK 를 회전해야 한다. 회전 과정에서 모든 자격증명이 새 KEK 로 재암호화되고, 각 단계는 audit_chain 에 hash-linked 로 기록된다. 회전 후에는 chain 무결성을 검증해 변조가 없었음을 증명하고 체크포인트를 남긴다.

## 현재 노출 상태 (정직)

- `audit_chain` 라이브러리 (`crates/openxgram-cli/src/audit.rs`) 와 `verify_chain` / `verify_checkpoints` 함수는 구현되어 있다. `AuditAction::Verify / Backfill / Checkpoint` enum 도 존재한다.
- 그러나 **현재 시점에는 `xgram audit` top-level CLI 명령이 노출되어 있지 않다** (Commands enum 미등록). `xgram vault rotate-kek` 도 아직 VaultCli 에 없다.
- 따라서 본 케이스의 회전·검증 흐름은 **후속 PR 작업** 이며, 이 문서는 **계약(contract)** 으로서 사용 흐름을 못 박는다. 라이브러리 함수 (`audit::run_audit`, `audit::verify`) 는 통합 테스트 / MCP / TUI 측에서 직접 호출해 검증 가능하다.

## 사전 준비

- `xgram init` 완료
- `XGRAM_KEYSTORE_PASSWORD` (현재 KEK)
- 회전 직전 cold backup 권장 (케이스 08 참조)

## 단계별 명령 시퀀스 (후속 PR 노출 예정 — 계약)

```bash
# 0) 안전망 — 회전 전 cold backup (현재 KEK 로 암호화)
xgram backup --to /tmp/pre-rotate-2026-05-04.bin

# 1) audit chain 무결성 사전 검증 (후속 PR — xgram audit verify)
#    현재는 라이브러리 호출 또는 통합테스트로 동등 검증
#    예정 명령: xgram audit verify

# 2) 새 KEK 후보로 KEK 회전 (후속 PR — xgram vault rotate-kek)
#    예정 명령:
#    XGRAM_KEYSTORE_PASSWORD=<old> XGRAM_KEYSTORE_PASSWORD_NEW=<new> \
#      xgram vault rotate-kek

# 3) 회전 결과 — 모든 vault row 가 새 KEK 로 재암호화 + audit row append
#    예정 명령: xgram audit verify

# 4) 체크포인트 — 회전 직후 master ECDSA 서명 체크포인트 기록
#    예정 명령: xgram audit checkpoint

# 5) 백필 — 과거 chain 에 missing hash 가 있으면 채우기
#    예정 명령: xgram audit backfill

# 6) 회전 후 평소처럼 vault 사용 (새 password 로 unwrap)
export XGRAM_KEYSTORE_PASSWORD="<new>"
xgram vault list
xgram vault get --key DISCORD_WEBHOOK_URL
```

## 기대 결과 (계약)

```
$ xgram audit verify
✓ chain ok        (rows=124, last=01HZ...)
✓ checkpoints ok  (count=4, last_signed_at=2026-05-04 14:33:21+09:00)

$ xgram vault rotate-kek
✓ KEK 회전 완료
  rows_reencrypted : 29
  audit_appended   : 30  (29 row + 1 rotate marker)
  duration         : 0.4s

$ xgram audit checkpoint
✓ 체크포인트 기록 (id=01HZ..., signature=...)
```

## 주의점

- **fallback 금지**: KEK 회전 도중 한 row 라도 재암호화 실패 시 전체 트랜잭션 abort — 부분 성공 묵인 금지.
- **롤백 가능**: 회전은 atomic 트랜잭션. 실패 시 자동 rollback. 그래도 회전 직전 cold backup 필수 (단계 0).
- **DB 변경 승인**: KEK 회전은 모든 vault row 를 변경하는 destructive — 마스터 명시적 호출 필수. password 두 개 (old/new) 환경변수로 명시 주입.
- **KST 시간대**: audit row.created_at, checkpoint.signed_at 모두 Asia/Seoul.
- 현재는 라이브러리 단계 — `xgram audit verify`, `xgram vault rotate-kek` top-level 명령 노출은 후속 PR. 그 전까지는 통합테스트 + MCP 도구로 호출.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` §8.5 KEK 회전 정책
- `docs/prd/PRD-OpenXgram-v1.md` audit_chain (hash-linked + 체크포인트 서명)
