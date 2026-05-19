#!/usr/bin/env bash
# OpenXgram quickstart — 한 줄 마법사.
#
# 사용:
#   curl -sSfL https://openxgram.org/quickstart.sh | bash
#
# 흐름:
#   1. xgram 미설치면 install.sh 자동 실행
#   2. 머신 연결 선택: [1] 새 노드(시드 신규) / [2] 기존 노드에 추가(원격 daemon 로그인)
#   3. 이메일 + 비밀번호 입력 (12자+) — 웹 GUI 로그인 자격
#   4. [1] 새 노드: alias + keystore 패스워드 → init → register (첫 사용자 → admin)
#      [2] 기존 노드: 원격 daemon URL → 이메일+비밀번호로 login → JWT 저장
#   5. (선택) 외부 채널 — Discord/Telegram
#   6. ~/.openxgram/.env 저장 (chmod 600)
#   7. daemon + agent 백그라운드 가동 (setsid)
#   8. 상태 확인 + 다음 명령 안내

set -euo pipefail

DATA_DIR="${XGRAM_DATA_DIR:-$HOME/.openxgram}"
ENV_FILE="$DATA_DIR/.env"
MANIFEST="$DATA_DIR/install-manifest.json"
DAEMON_GUI_PORT="${XGRAM_DAEMON_GUI_PORT:-47302}"

echo ""
echo "═══════════════════════════════════════════════════"
echo "  OpenXgram quickstart"
echo "  데이터 디렉토리: $DATA_DIR"
echo "═══════════════════════════════════════════════════"
echo ""

# 1. xgram 바이너리 — 미설치면 install.sh
if ! command -v xgram >/dev/null 2>&1; then
    echo "→ xgram 미설치 — install.sh 자동 실행"
    curl -sSfL https://openxgram.org/install.sh | sh
    export PATH="$HOME/.local/bin:$PATH"
fi
XGRAM_VERSION=$(xgram --version 2>&1 | head -1)
echo "  xgram : $XGRAM_VERSION"
echo ""

# 2. 머신 연결 선택 (manifest 없을 때만)
MODE="1"
if [ ! -f "$MANIFEST" ]; then
    echo "── 이 머신을 어떻게 사용하시겠어요? ──"
    echo "  [1] 새 노드로 시작 (시드 신규 발급, 독립 신원·메모리)"
    echo "  [2] 기존 노드에 머신 추가 (다른 머신 계정으로 원격 daemon 사용)"
    echo ""
    read -r -p "선택 [1/2] (Enter = 1): " MODE
    MODE=${MODE:-1}
    echo ""
fi

# 3. 이메일 + 비밀번호 (양쪽 모드 공통)
read -r -p "이메일: " EMAIL
while :; do
    read -r -s -p "웹 GUI 비밀번호 (최소 12자): " GUI_PASSWORD
    echo ""
    if [ ${#GUI_PASSWORD} -ge 12 ]; then break; fi
    echo "  ✗ 최소 12자 — 다시 입력"
done
echo ""

# ── [2] 기존 노드 추가 ──────────────────────────────────────
if [ "$MODE" = "2" ]; then
    read -r -p "원격 daemon URL (예: https://other-machine.tailXXXX.ts.net): " REMOTE_URL
    REMOTE_URL=${REMOTE_URL%/}
    if [ -z "$REMOTE_URL" ]; then
        echo "✗ URL 입력 필수" >&2
        exit 1
    fi

    echo "→ 원격 daemon 에 로그인: $REMOTE_URL/v1/auth/login"
    LOGIN_RESP=$(curl -sS -X POST "$REMOTE_URL/v1/auth/login" \
        -H "Content-Type: application/json" \
        --data-binary @<(printf '{"email":"%s","password":"%s"}' "$EMAIL" "$GUI_PASSWORD") \
        -w '\n%{http_code}')
    HTTP_CODE=$(echo "$LOGIN_RESP" | tail -1)
    BODY=$(echo "$LOGIN_RESP" | sed '$d')
    if [ "$HTTP_CODE" != "200" ]; then
        echo "✗ 로그인 실패 (HTTP $HTTP_CODE)" >&2
        echo "  응답: $BODY" >&2
        exit 1
    fi
    JWT=$(echo "$BODY" | grep -o '"jwt_token":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -z "$JWT" ]; then
        echo "✗ JWT 추출 실패 — 응답 형식 확인: $BODY" >&2
        exit 1
    fi

    mkdir -p "$DATA_DIR"
    umask 077
    {
        echo "# OpenXgram quickstart (원격 노드 모드)"
        echo "export XGRAM_DAEMON_URL='$REMOTE_URL'"
        echo "export XGRAM_GUI_JWT='$JWT'"
    } > "$ENV_FILE"
    chmod 600 "$ENV_FILE"

    echo ""
    echo "✓ 원격 노드 연결됨 — 이 머신의 xgram CLI/GUI는 원격 daemon 사용"
    echo "  (이 머신에는 daemon 띄우지 않음)"
    echo ""
    echo "다음 명령:"
    echo "  웹 GUI 열기: 원격 머신의 https://<host>.tailXXXX.ts.net/gui/"
    echo "  → 같은 이메일/비밀번호로 로그인하면 머신 어디서든 동일 자격"
    echo ""
    exit 0
fi

# ── [1] 새 노드 (default) ───────────────────────────────────
# 4. xgram init — manifest 없으면 alias + 패스워드 prompt 후 init
if [ ! -f "$MANIFEST" ]; then
    DEFAULT_ALIAS=$(hostname -s 2>/dev/null || echo "my-machine")
    read -r -p "이 머신 alias (default: $DEFAULT_ALIAS): " ALIAS
    ALIAS=${ALIAS:-$DEFAULT_ALIAS}

    while :; do
        read -r -s -p "keystore 패스워드 (최소 12자): " PASSWORD
        echo ""
        if [ ${#PASSWORD} -ge 12 ]; then break; fi
        echo "  ✗ 최소 12자 — 다시 입력"
    done

    export XGRAM_KEYSTORE_PASSWORD="$PASSWORD"
    export XGRAM_INIT_SKIP_SEED_BACKUP_CONFIRM=1  # 비-TTY 환경 호환 (시드는 화면에 뜸)

    echo ""
    echo "→ xgram init --alias '$ALIAS'"
    xgram init --alias "$ALIAS"
else
    echo "→ 기존 install-manifest 발견 — init 건너뜀"
    echo "  $(grep -o '"alias": *"[^"]*"' "$MANIFEST" | head -1)"
    read -r -s -p "keystore 패스워드 입력 (저장된 봇 가동용): " PASSWORD
    echo ""
    export XGRAM_KEYSTORE_PASSWORD="$PASSWORD"
fi
echo ""

# 5. 외부 채널 — Enter 로 skip.
echo "── 외부 채널 (모두 선택 — Enter 로 skip) ──"
read -r -p "Discord webhook URL (Enter skip): " DISCORD_WEBHOOK || DISCORD_WEBHOOK=""
read -r -p "Discord bot token   (Enter skip): " DISCORD_BOT_TOKEN || DISCORD_BOT_TOKEN=""
read -r -p "Discord channel id  (Enter skip): " DISCORD_CHANNEL_ID || DISCORD_CHANNEL_ID=""
read -r -p "Telegram bot token  (Enter skip): " TELEGRAM_BOT_TOKEN || TELEGRAM_BOT_TOKEN=""
read -r -p "Telegram chat id    (Enter skip): " TELEGRAM_CHAT_ID || TELEGRAM_CHAT_ID=""
ANTHROPIC_API_KEY=""
echo ""

# 6. .env 저장 (chmod 600)
umask 077
mkdir -p "$DATA_DIR"
{
    echo "# OpenXgram quickstart — 다음 세션부터 source 이 파일로 환경 복원"
    echo "export XGRAM_KEYSTORE_PASSWORD='$PASSWORD'"
    [ -n "$DISCORD_WEBHOOK"     ] && echo "export XGRAM_DISCORD_WEBHOOK_URL='$DISCORD_WEBHOOK'"
    [ -n "$DISCORD_BOT_TOKEN"   ] && echo "export XGRAM_DISCORD_BOT_TOKEN='$DISCORD_BOT_TOKEN'"
    [ -n "$DISCORD_CHANNEL_ID"  ] && echo "export XGRAM_DISCORD_CHANNEL_ID='$DISCORD_CHANNEL_ID'"
    [ -n "$TELEGRAM_BOT_TOKEN"  ] && echo "export XGRAM_TELEGRAM_BOT_TOKEN='$TELEGRAM_BOT_TOKEN'"
    [ -n "$TELEGRAM_CHAT_ID"    ] && echo "export XGRAM_TELEGRAM_CHAT_ID='$TELEGRAM_CHAT_ID'"
    [ -n "$ANTHROPIC_API_KEY"   ] && echo "export XGRAM_ANTHROPIC_API_KEY='$ANTHROPIC_API_KEY'"
} > "$ENV_FILE"
chmod 600 "$ENV_FILE"
echo "→ 비밀 저장: $ENV_FILE (chmod 600)"
echo ""

# 7. 기존 가동 프로세스 종료 (idempotent)
pkill -f "xgram daemon" 2>/dev/null || true
pkill -f "xgram agent"  2>/dev/null || true
sleep 1

# 8. daemon — setsid 로 새 세션
echo "→ daemon 가동"
setsid bash -c "
    source '$ENV_FILE'
    exec xgram daemon
" > "$DATA_DIR/daemon.log" 2>&1 < /dev/null &
disown
sleep 3

# 9. 웹 GUI 사용자 등록 (이메일+비밀번호) — daemon /v1/auth/register
echo "→ 웹 GUI 사용자 등록"
REG_RESP=$(curl -sS -X POST "http://127.0.0.1:${DAEMON_GUI_PORT}/v1/auth/register" \
    -H "Content-Type: application/json" \
    --data-binary @<(printf '{"email":"%s","password":"%s","alias":"%s"}' "$EMAIL" "$GUI_PASSWORD" "${ALIAS:-}") \
    -w '\n%{http_code}' || echo $'\n000')
HTTP_CODE=$(echo "$REG_RESP" | tail -1)
BODY=$(echo "$REG_RESP" | sed '$d')
case "$HTTP_CODE" in
    200)
        ROLE=$(echo "$BODY" | grep -o '"role":"[^"]*"' | head -1 | cut -d'"' -f4)
        echo "  ✓ 가입 완료 (role=$ROLE)"
        ;;
    400)
        if echo "$BODY" | grep -q "이미 가입"; then
            echo "  → 이미 등록된 이메일 — 기존 계정 유지"
        else
            echo "  ✗ 가입 실패: $BODY"
        fi
        ;;
    *)
        echo "  ✗ 가입 응답: HTTP $HTTP_CODE — $BODY"
        echo "  daemon 가동 로그: $DATA_DIR/daemon.log"
        ;;
esac
echo ""

# 10. agent — Discord/Telegram 채널 토큰 있을 때만 가동
if [ -n "$DISCORD_WEBHOOK" ] || [ -n "$TELEGRAM_BOT_TOKEN" ]; then
    echo "→ agent 가동 (외부 채널 forward)"
    AGENT_ARGS=()
    [ -n "$DISCORD_WEBHOOK" ]    && AGENT_ARGS+=(--discord-webhook-url "$DISCORD_WEBHOOK")
    [ -n "$DISCORD_BOT_TOKEN" ]  && AGENT_ARGS+=(--discord-bot-token "$DISCORD_BOT_TOKEN")
    [ -n "$DISCORD_CHANNEL_ID" ] && AGENT_ARGS+=(--discord-channel-id "$DISCORD_CHANNEL_ID")
    setsid bash -c "
        source '$ENV_FILE'
        exec xgram agent ${AGENT_ARGS[*]@Q}
    " > "$DATA_DIR/agent.log" 2>&1 < /dev/null &
    disown
    sleep 2
fi

# 11. 상태 확인
echo ""
echo "── 가동 상태 ──"
DAEMON_PID=$(pgrep -f "xgram daemon" | head -1 || echo "")
AGENT_PID=$(pgrep -f "xgram agent" | head -1 || echo "")
if [ -n "$DAEMON_PID" ]; then echo "  ✓ daemon  PID $DAEMON_PID  (log: $DATA_DIR/daemon.log)"; else echo "  ✗ daemon  미가동 — 로그 확인: $DATA_DIR/daemon.log"; fi
if [ -n "$AGENT_PID"  ]; then echo "  ✓ agent   PID $AGENT_PID  (log: $DATA_DIR/agent.log)";   else echo "  - agent   미가동 (외부 채널 토큰 없어 skip 됐을 수 있음)"; fi
echo ""

# 12. 안내
echo "═══════════════════════════════════════════════════"
echo "  ✓ OpenXgram 가동 완료"
echo ""
echo "  웹 GUI 로그인 자격:"
echo "    이메일   : $EMAIL"
echo "    비밀번호 : (입력하신 GUI 비밀번호)"
echo ""
echo "  다음 명령:"
echo "    xgram peer send --alias <대상> --body \"메시지\"   # 메시지 보내기"
echo "    xgram bot register <name>                            # 추가 봇 등록"
echo "    xgram gui                                            # 웹 GUI (Tailscale Funnel URL 자동 오픈)"
echo ""
echo "  환경 복원 (새 세션):"
echo "    source $ENV_FILE"
echo "═══════════════════════════════════════════════════"
