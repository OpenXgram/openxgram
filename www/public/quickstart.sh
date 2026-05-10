#!/usr/bin/env bash
# OpenXgram quickstart — 한 줄 마법사.
#
# 사용:
#   curl -sSfL https://openxgram.org/quickstart.sh | bash
#
# 흐름:
#   1. xgram 미설치면 install.sh 자동 실행
#   2. alias / keystore 패스워드 입력 (기존 init 있으면 skip)
#   3. Discord webhook / Telegram bot / Anthropic API 키 입력 (Enter 로 skip)
#   4. ~/.openxgram/.env 에 비밀 저장 (chmod 600)
#   5. daemon + agent 백그라운드 가동 (setsid 로 진짜 detach)
#   6. 상태 확인 + 다음 명령 안내

set -euo pipefail

DATA_DIR="${XGRAM_DATA_DIR:-$HOME/.openxgram}"
ENV_FILE="$DATA_DIR/.env"
MANIFEST="$DATA_DIR/install-manifest.json"

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

# 2. init — manifest 없으면 alias + 패스워드 prompt 후 init
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

# 3. 외부 채널 / LLM 토큰 — Enter 로 skip
echo "── 외부 채널 / LLM 연동 (모두 선택 — Enter 로 skip) ──"
read -r -p "Discord webhook URL (Enter skip): " DISCORD_WEBHOOK || DISCORD_WEBHOOK=""
read -r -p "Discord bot token   (Enter skip): " DISCORD_BOT_TOKEN || DISCORD_BOT_TOKEN=""
read -r -p "Discord channel id  (Enter skip): " DISCORD_CHANNEL_ID || DISCORD_CHANNEL_ID=""
read -r -p "Telegram bot token  (Enter skip): " TELEGRAM_BOT_TOKEN || TELEGRAM_BOT_TOKEN=""
read -r -p "Telegram chat id    (Enter skip): " TELEGRAM_CHAT_ID || TELEGRAM_CHAT_ID=""
read -r -p "Anthropic API key   (Enter skip): " ANTHROPIC_API_KEY || ANTHROPIC_API_KEY=""
echo ""

# 4. .env 저장 (chmod 600 — 평문이지만 home dir 보호 가정. vault 이전은 후속)
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

# 5. 기존 가동 프로세스 종료 (재실행 시 idempotent)
pkill -f "xgram daemon" 2>/dev/null || true
pkill -f "xgram agent"  2>/dev/null || true
sleep 1

# 6. daemon — setsid 로 새 세션, 부모 종료해도 살아남음
echo "→ daemon 가동"
setsid bash -c "
    source '$ENV_FILE'
    exec xgram daemon
" > "$DATA_DIR/daemon.log" 2>&1 < /dev/null &
disown
sleep 2

# 7. agent — discord/telegram/anthropic 키 있을 때만
if [ -n "$DISCORD_WEBHOOK" ] || [ -n "$TELEGRAM_BOT_TOKEN" ] || [ -n "$ANTHROPIC_API_KEY" ]; then
    echo "→ agent 가동 (외부 채널 forward + LLM 응답)"
    AGENT_ARGS=()
    [ -n "$DISCORD_WEBHOOK" ]    && AGENT_ARGS+=(--discord-webhook-url "$DISCORD_WEBHOOK")
    [ -n "$DISCORD_BOT_TOKEN" ]  && AGENT_ARGS+=(--discord-bot-token "$DISCORD_BOT_TOKEN")
    [ -n "$DISCORD_CHANNEL_ID" ] && AGENT_ARGS+=(--discord-channel-id "$DISCORD_CHANNEL_ID")
    [ -n "$ANTHROPIC_API_KEY" ]  && AGENT_ARGS+=(--anthropic-api-key "$ANTHROPIC_API_KEY")
    setsid bash -c "
        source '$ENV_FILE'
        exec xgram agent ${AGENT_ARGS[*]@Q}
    " > "$DATA_DIR/agent.log" 2>&1 < /dev/null &
    disown
    sleep 2
fi

# 8. 상태 확인
echo ""
echo "── 가동 상태 ──"
DAEMON_PID=$(pgrep -f "xgram daemon" | head -1 || echo "")
AGENT_PID=$(pgrep -f "xgram agent" | head -1 || echo "")
if [ -n "$DAEMON_PID" ]; then echo "  ✓ daemon  PID $DAEMON_PID  (log: $DATA_DIR/daemon.log)"; else echo "  ✗ daemon  미가동 — 로그 확인: $DATA_DIR/daemon.log"; fi
if [ -n "$AGENT_PID"  ]; then echo "  ✓ agent   PID $AGENT_PID  (log: $DATA_DIR/agent.log)";   else echo "  - agent   미가동 (외부 채널/LLM 토큰 없어 skip 됐을 수 있음)"; fi
echo ""

# 9. 안내
echo "═══════════════════════════════════════════════════"
echo "  ✓ OpenXgram 가동 완료"
echo ""
echo "  다음 명령:"
echo "    xgram peer send --alias <대상> --body \"메시지\"   # 메시지 보내기"
echo "    xgram bot register <name>                            # 추가 봇 등록"
echo "    xgram pair-desktop                                   # 다른 머신 페어링 URL"
echo "    xgram gui                                            # 데스크탑 GUI"
echo ""
echo "  환경 복원 (새 세션):"
echo "    source $ENV_FILE"
echo "═══════════════════════════════════════════════════"
