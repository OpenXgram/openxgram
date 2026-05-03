#!/usr/bin/env bash
# OpenXgram INJECT — 클립보드에 컨텍스트 주입 프롬프트 복사
#
# 사용법:
#   ./.handoff/inject.sh         # 클립보드에 복사
#   ./.handoff/inject.sh --print # stdout으로 출력 (파이프 가능)
#
# 마스터가 alias로 등록 권장:
#   alias xginject='/home/llm/projects/openxgram/.handoff/inject.sh'

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INJECT_FILE="${SCRIPT_DIR}/INJECT.md"

if [[ ! -f "$INJECT_FILE" ]]; then
  echo "ERROR: INJECT.md가 ${SCRIPT_DIR}에 없다" >&2
  exit 1
fi

if [[ "${1:-}" == "--print" ]]; then
  cat "$INJECT_FILE"
  exit 0
fi

# OS별 클립보드 명령 자동 감지 (fallback 금지 — 못 찾으면 raise)
if command -v pbcopy >/dev/null 2>&1; then
  cat "$INJECT_FILE" | pbcopy
  echo "INJECT 프롬프트 클립보드 복사 완료 (macOS pbcopy)"
elif command -v xclip >/dev/null 2>&1; then
  cat "$INJECT_FILE" | xclip -selection clipboard
  echo "INJECT 프롬프트 클립보드 복사 완료 (Linux xclip)"
elif command -v wl-copy >/dev/null 2>&1; then
  cat "$INJECT_FILE" | wl-copy
  echo "INJECT 프롬프트 클립보드 복사 완료 (Wayland wl-copy)"
elif [[ -n "${SSH_CONNECTION:-}" ]]; then
  # SSH 환경 — OSC52 escape sequence로 클립보드 전송
  printf '\033]52;c;%s\007' "$(base64 -w0 < "$INJECT_FILE")"
  echo "INJECT 프롬프트 OSC52로 전송 (SSH 터미널이 OSC52 지원해야 함)"
else
  echo "ERROR: 클립보드 도구를 찾지 못함 (pbcopy/xclip/wl-copy 없음, SSH도 아님)" >&2
  echo "       --print 옵션으로 stdout 출력 후 수동 복사 가능" >&2
  exit 1
fi

echo ""
echo "다음 행동:"
echo "  1. 기동 중인 claude 세션 입력창으로 이동"
echo "  2. Cmd+V (또는 Ctrl+V)로 붙여넣기"
echo "  3. 엔터"
echo "  4. claude가 컨텍스트 흡수 후 보고 -> 작업 진행"
