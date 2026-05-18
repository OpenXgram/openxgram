# 08 — age multi-recipient backup + 복구

> 한 줄 요약: ChaCha20-Poly1305 cold backup 으로 OpenXgram 데이터를 밀봉하고, 다른 머신에서 password 로 복원하거나 merge 한다.

## 시나리오

마스터는 GCP 머신이 갑자기 사라질 가능성에 대비해 매주 일요일 03:00 KST 에 OpenXgram 전체를 cold backup 하고, 별도 보관소로 push 한다. 새 머신을 받으면 이 backup 으로 복원해 신원·기억·자격증명을 그대로 잇는다. age multi-recipient(여러 공개키로 동시 봉인) 는 후속 PR 이고 현재는 keystore password 기반 ChaCha20-Poly1305 cold backup 이 안정 노출 상태.

## 현재 노출 상태 (정직)

- `xgram backup` / `xgram restore` / `xgram backup-install` / `xgram backup-uninstall` / `xgram backup-push` 은 모두 Commands enum 에 등록 — 사용 가능.
- `crates/openxgram-cli/src/age_backup.rs` 가 존재하지만 (`encrypt_with_passphrase` 함수) **age multi-recipient (여러 X25519 수신자) CLI 노출은 후속 PR**. 현재 cold backup 은 ChaCha20-Poly1305 + Argon2id (single password) 한 가지.

## 사전 준비

- `xgram init` 완료
- `XGRAM_KEYSTORE_PASSWORD` export — backup 봉인 / restore 해제에 사용
- (옵션) Discord webhook · Telegram bot 토큰 vault 등록 (backup-push 용)

## 단계별 명령 시퀀스

```bash
# 1) 즉시 cold backup — DB + keystore + manifest 를 단일 .bin 으로 봉인
xgram backup --to /backups/openxgram-2026-05-04.bin

# 2) systemd timer 로 자동화 (매주 일요일 03:00 KST)
xgram backup-install \
  --backup-dir /backups \
  --on-calendar "Sun 03:00:00"
systemctl --user daemon-reload
systemctl --user enable --now openxgram-backup.timer

# 3) backup 파일을 외부로 push — Discord 또는 Telegram 으로 통계 + 첨부
#    (대용량은 Discord 25MB 제한 주의 — 통계만 보내고 파일은 별도 보관소)
xgram backup-push \
  --session-id <SID> \
  --target discord
xgram backup-push \
  --session-id <SID> \
  --target telegram

# 4) 새 머신 — restore (전체 덮어쓰기)
export XGRAM_KEYSTORE_PASSWORD="<원래 password>"
xgram restore \
  --input /backups/openxgram-2026-05-04.bin \
  --target-dir ~/.openxgram

# 5) 새 머신 — restore (merge 모드, 기존 데이터에 합치기)
xgram restore \
  --input /backups/openxgram-2026-05-04.bin \
  --target-dir ~/.openxgram \
  --merge

# 6) 복원 후 검증
xgram doctor
xgram session list
xgram vault list

# 7) timer 제거가 필요할 때
systemctl --user disable --now openxgram-backup.timer
xgram backup-uninstall
```

## 기대 결과

```
$ xgram backup --to /backups/openxgram-2026-05-04.bin
✓ cold backup 생성
  source       : ~/.openxgram
  target       : /backups/openxgram-2026-05-04.bin
  bytes_written: 18,432,011
  encryption   : ChaCha20-Poly1305 + Argon2id
  signed_at    : 2026-05-04 14:33:21+09:00

$ xgram restore --input /backups/openxgram-2026-05-04.bin --target-dir ~/.openxgram
✓ cold backup 복원 완료
  source        : /backups/openxgram-2026-05-04.bin
  target_dir    : /home/llm/.openxgram
  bytes_restored: 18,432,011

$ xgram backup-install --backup-dir /backups --on-calendar "Sun 03:00:00"
✓ systemd backup units 생성
  service: ~/.config/systemd/user/openxgram-backup.service
  timer  : ~/.config/systemd/user/openxgram-backup.timer
주의: XGRAM_KEYSTORE_PASSWORD 는 systemd-creds 또는 EnvironmentFile 로 별도 주입.
```

## 주의점

- **fallback 금지**: restore 시 password 누락 / 잘못된 password 는 raise. 키 없이 일부만 복원하는 fallback 절대 없음.
- **롤백 가능**: `restore` (덮어쓰기) 는 destructive — target_dir 가 비어있지 않으면 사전 backup 필수. `--merge` 는 덜 위험하지만 충돌 행이 발생하면 raise.
- **DB 변경 승인**: restore 는 대량 INSERT/UPDATE — 마스터 명시 호출만. systemd timer 자동화도 마스터 승인 후 enable.
- **KST 시간대**: backup 파일명·signed_at·systemd OnCalendar 모두 Asia/Seoul. `Sun 03:00:00` 은 KST 03:00 (systemd 는 시스템 TZ = Asia/Seoul 가정).
- age multi-recipient (여러 수신자 공개키 봉인) 는 후속 PR. 현재는 single password 기반 cold backup.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` Cold Backup 섹션 (ChaCha20-Poly1305 + Argon2id)
- `docs/prd/PRD-OpenXgram-v1.md` Backup Push (Discord / Telegram outbound)
