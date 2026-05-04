# 07 — Retention 정책 적용 (90 일 L0 압축)

> 한 줄 요약: 90 일 이상 된 L0 messages 를 episode 요약으로 압축하고 임베딩만 남겨 SQLite 파일 크기를 관리한다.

## 시나리오

마스터의 OpenXgram 노드는 매일 수천 건의 messages 를 받는다. 6 개월 뒤면 SQLite 파일이 수 GB 로 부풀고 sqlite-vec KNN 도 느려진다. retention 정책으로 90 일 이상 된 L0 raw 메시지는 episode 요약·임베딩만 남기고 본문(body)을 archive blob 으로 옮기거나 삭제해 핫 storage 를 가볍게 유지한다.

## 현재 노출 상태 (정직)

- `crates/openxgram-cli/src/retention.rs` 에 retention 로직(11 KB)이 구현되어 있다.
- 그러나 **현재 시점에는 `xgram retention` top-level CLI 명령이 Commands enum 에 등록되어 있지 않다**. 따라서 본 케이스의 명령 시퀀스는 **후속 PR 노출 예정 계약** 이다.
- 그 전까지는 daemon 의 야간 cron / 통합 테스트 / MCP 도구로 retention 함수를 호출 가능.

## 사전 준비

- `xgram init` + `xgram daemon` 가동 (또는 `xgram daemon-install` 로 systemd user unit 등록)
- 회전 전 cold backup 권장 (케이스 08)
- L1 episodes 가 충분히 쌓여 있어야 압축 가치 있음 (먼저 `xgram session reflect-all`)

## 단계별 명령 시퀀스 (후속 PR 노출 예정 — 계약)

```bash
# 0) 안전망 — retention 실행 전 cold backup
xgram backup --to /tmp/pre-retention-2026-05-04.bin

# 1) reflection 일괄 — episodes 가 모든 messages 를 커버하는지 보장
xgram session reflect-all

# 2) dry-run 으로 영향 범위 미리 보기 (예정 명령)
#    xgram retention run --age-days 90 --dry-run

# 3) 실제 retention 실행 — 90 일 이상 messages 의 body 압축 (예정 명령)
#    xgram retention run --age-days 90

# 4) (옵션) episode 단위 compact — 동일 session 의 episodes 머지 (예정 명령)
#    xgram retention compact --session-id <SID>

# 5) DB 진단으로 디스크 사용량 변화 확인
xgram doctor --json
xgram dump --kind sizes

# 6) cron 자동화 — daemon 의 reflection cron 과 별도 schedule
xgram daemon-install \
  --bind 127.0.0.1:7300 \
  --reflection-cron "0 0 15 * * *"
# retention cron 표현식 노출은 후속 PR
```

## 기대 결과 (계약)

```
$ xgram retention run --age-days 90 --dry-run
[dry-run] retention 영향 범위
  messages_to_compact : 12,431
  bytes_freed         : ~84 MB
  episodes_unchanged  : 1,028
  embeddings_kept     : 12,431

$ xgram retention run --age-days 90
✓ retention 완료
  messages_compacted : 12,431
  bytes_freed        : 86,234,112
  duration           : 4.2s
  audit_appended     : 1
```

## 주의점

- **fallback 금지**: retention 도중 임베딩 누락 message 발견 시 silently skip 금지 — raise. 누락 임베딩은 먼저 backfill.
- **롤백 가능**: retention 은 destructive (본문 삭제). 단계 0 의 cold backup 으로 복구 가능. dry-run 단계 1 회 필수.
- **DB 변경 승인**: retention 은 대량 삭제·UPDATE — 마스터 명시 호출만. 자동 cron 활성화도 마스터 승인 후.
- **KST 시간대**: `--age-days 90` 의 cutoff 는 `kst_now() - 90 days` — UTC 가 아닌 Asia/Seoul.
- L4 traits / pinned L2 memories 는 retention 에서 제외 (정체성·중요 사실 보호).

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` §10 메모리 retention 정책
- `docs/prd/PRD-OpenXgram-v1.md` §17 reflection → compaction 파이프라인
