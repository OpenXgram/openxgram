#!/usr/bin/env bash
# scripts/deploy-www.sh — openxgram.org 배포 (이 서버가 nginx origin 인 환경 전용).
#
# 흐름:
#   1. www/ 빌드 (vite)
#   2. dist/ → /var/www/openxgram-www/ 로 rsync (--delete, atomic-ish)
#   3. 라이브 사이트의 main JS 해시가 빌드 결과와 일치하는지 검증
#
# Usage:
#   scripts/deploy-www.sh                  # 빌드 + 배포 + 검증
#   scripts/deploy-www.sh --dry-run        # rsync 시뮬레이션만
#   scripts/deploy-www.sh --skip-build     # 이미 빌드된 dist/ 사용
#   scripts/deploy-www.sh --skip-verify    # 라이브 검증 생략
#
# 환경:
#   OXG_WEB_ROOT  — 기본 /var/www/openxgram-www (override 가능)
#   OXG_PUBLIC_URL — 검증용 URL prefix (기본 https://openxgram.org)
#
# 절대 규칙: silent fallback 금지 — 빌드/rsync/검증 실패 시 즉시 raise.

set -euo pipefail

DRY_RUN=0
SKIP_BUILD=0
SKIP_VERIFY=0
WEB_ROOT="${OXG_WEB_ROOT:-/var/www/openxgram-www}"
PUBLIC_URL="${OXG_PUBLIC_URL:-https://openxgram.org}"

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --skip-verify) SKIP_VERIFY=1; shift ;;
    -h|--help)
      sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

# 항상 repo 의 www 디렉토리에서 실행 (스크립트 위치 기준).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WWW_DIR="$SCRIPT_DIR/../www"
cd "$WWW_DIR"

if [ ! -d "$WEB_ROOT" ]; then
  echo "✗ web root 없음: $WEB_ROOT (OXG_WEB_ROOT 로 override)" >&2
  exit 3
fi
if [ ! -w "$WEB_ROOT" ]; then
  echo "✗ web root 쓰기 권한 없음: $WEB_ROOT" >&2
  echo "  현재 사용자: $(whoami) — 소유자/권한 확인" >&2
  exit 3
fi

if [ "$SKIP_BUILD" -eq 0 ]; then
  echo "→ npm run build (vite)"
  npm run build
fi

if [ ! -d dist ]; then
  echo "✗ dist/ 없음 — 빌드 먼저 (또는 --skip-build 해제)" >&2
  exit 4
fi

RSYNC_FLAGS="-av --delete"
if [ "$DRY_RUN" -eq 1 ]; then
  RSYNC_FLAGS="$RSYNC_FLAGS --dry-run"
  echo "→ [DRY RUN] rsync $RSYNC_FLAGS dist/ → $WEB_ROOT/"
else
  echo "→ rsync dist/ → $WEB_ROOT/"
fi
# shellcheck disable=SC2086
rsync $RSYNC_FLAGS dist/ "$WEB_ROOT/"

if [ "$DRY_RUN" -eq 1 ]; then
  echo "✓ dry-run 종료. 실제 배포는 --dry-run 빼고 재실행."
  exit 0
fi

# 빌드 결과의 main JS 해시 추출.
LOCAL_JS=$(grep -oE '/assets/main-[A-Za-z0-9_-]+\.js' dist/index.html | head -1 || true)
if [ -z "$LOCAL_JS" ]; then
  echo "⚠ dist/index.html 에서 main JS 해시 추출 실패 — 검증 생략" >&2
  SKIP_VERIFY=1
fi

if [ "$SKIP_VERIFY" -eq 0 ]; then
  echo "→ 라이브 검증: $PUBLIC_URL (예상 $LOCAL_JS)"
  TMPF=$(mktemp)
  for try in 1 2 3 4 5; do
    if curl -sSfL -H "cache-control: no-cache" "$PUBLIC_URL/?_cb=$(date +%s)" -o "$TMPF"; then
      LIVE_JS=$(grep -oE '/assets/main-[A-Za-z0-9_-]+\.js' "$TMPF" | head -1 || true)
      if [ "$LIVE_JS" = "$LOCAL_JS" ]; then
        rm -f "$TMPF"
        echo "✓ 라이브 일치: $LIVE_JS"
        break
      fi
      echo "  [$try/5] 라이브: $LIVE_JS — 일치 대기 (Cloudflare/edge 전파)"
    else
      echo "  [$try/5] $PUBLIC_URL HTTP 실패"
    fi
    if [ "$try" -eq 5 ]; then
      rm -f "$TMPF"
      echo "✗ 5회 시도 후에도 라이브 해시가 새 빌드와 다름. 직접 확인 필요." >&2
      exit 5
    fi
    sleep 4
  done
fi

echo "✓ 배포 완료"
