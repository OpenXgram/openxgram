#!/usr/bin/env bash
# rc.238 — inbound_processor stuck watchdog.
#
# 근본 문제: inbound_processor (daemon.rs 1s tick) 가 어떤 envelope 처리 중 hang 하면
# tick 전체가 멈추고 모든 후속 inbound 가 막힌다 (received_count 고정, ACK 0).
# daemon restart 로만 복구됨 — 반복 발생.
#
# 이 스크립트: health 의 last_inbound_tick_secs_ago 를 확인.
#   - tick 가 STALE_THRESHOLD(120s)+ 정체 → stuck 으로 판단 → systemctl --user restart.
#   - tick 정상(작은 값) → 아무것도 안 함 (false positive 방지).
#   - health 응답 자체 실패 → 별도 처리: 응답 없으면 daemon 죽었을 가능성 → restart.
#
# cron 등록 (2분):  */2 * * * * /home/llm/projects/starian-set/openxgram/scripts/inbound-watchdog.sh >> ~/.openxgram/inbound-watchdog.log 2>&1
set -uo pipefail

# health endpoint — systemd unit 의 --bind 와 일치 (Tailscale IP). 변경 시 env 로 override.
HEALTH_URL="${XGRAM_HEALTH_URL:-http://100.101.237.9:47300/v1/health}"
SERVICE="${XGRAM_SERVICE:-openxgram-sidecar.service}"
STALE_THRESHOLD="${XGRAM_INBOUND_STALE_SECS:-120}"
TS="$(date '+%Y-%m-%d %H:%M:%S %Z')"

# 1. health 조회 (5초 timeout)
BODY="$(curl -fsS --max-time 5 "$HEALTH_URL" 2>/dev/null)"
CURL_RC=$?

if [[ $CURL_RC -ne 0 || -z "$BODY" ]]; then
  # health 응답 없음 → daemon down 또는 transport bind 실패. restart 로 복구.
  echo "[$TS] WARN health 응답 실패 (curl_rc=$CURL_RC) — $SERVICE restart 시도"
  systemctl --user restart "$SERVICE"
  echo "[$TS] RESTART 완료 (health 무응답)"
  exit 0
fi

# 2. last_inbound_tick_secs_ago 파싱 (jq 있으면 jq, 없으면 grep fallback).
if command -v jq >/dev/null 2>&1; then
  AGO="$(printf '%s' "$BODY" | jq -r '.last_inbound_tick_secs_ago // empty' 2>/dev/null)"
else
  AGO="$(printf '%s' "$BODY" | grep -oE '"last_inbound_tick_secs_ago"[[:space:]]*:[[:space:]]*[0-9]+' | grep -oE '[0-9]+$' | head -n1)"
fi

# 3. 필드 없음(미tick / 구버전 daemon) → 보수적으로 skip (정보 부족 시 restart 안 함).
if [[ -z "$AGO" ]]; then
  echo "[$TS] INFO last_inbound_tick_secs_ago 필드 없음 (미tick 또는 구버전) — skip"
  exit 0
fi

# 4. 임계치 비교.
if [[ "$AGO" -gt "$STALE_THRESHOLD" ]]; then
  echo "[$TS] STUCK inbound_processor tick ${AGO}s 정체 (> ${STALE_THRESHOLD}s) — $SERVICE restart"
  systemctl --user restart "$SERVICE"
  echo "[$TS] RESTART 완료 (stuck 복구)"
else
  echo "[$TS] OK inbound tick ${AGO}s ago (정상, < ${STALE_THRESHOLD}s)"
fi
