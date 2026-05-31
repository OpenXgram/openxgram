#!/usr/bin/env sh
# OpenXgram installer — pre-built binary 우선, 실패 시 cargo 빌드 fallback (silent 금지).
#
# Usage:
#   curl -sSfL https://openxgram.org/install.sh | sh
#   curl -sSfL https://openxgram.org/install.sh | sh -s -- --version v0.2.0-alpha.1
#   curl -sSfL https://openxgram.org/install.sh | sh -s -- --dry-run
#
# Privacy: 이 스크립트는 GitHub Releases asset 만 download + SHA256 검증 후 install.
# 텔레메트리·통계·외부 보고 0. Source:
#   https://github.com/OpenXgram/openxgram/blob/main/www/install.sh

set -eu

REPO="OpenXgram/openxgram"
VERSION="${OPENXGRAM_VERSION:-latest}"
INSTALL_DIR="${OPENXGRAM_INSTALL_DIR:-}"
DRY_RUN="0"
# rc.169+: Claude Code 가 깔려있으면 (~/.claude.json 존재) FULL 자동 활성 — 메신저 본질.
if [ -f "$HOME/.claude.json" ]; then
  FULL="1"
else
  FULL="0"
fi

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --dry-run) DRY_RUN="1"; shift 1 ;;
    --full) FULL="1"; shift 1 ;;
    --help|-h)
      cat <<EOF
OpenXgram installer

Options:
  --version <tag>      특정 release tag (default: latest pre-release/release)
  --install-dir <dir>  설치 위치 (default: ~/.local/bin)
  --dry-run            검증만 — 실제 설치는 하지 않음
  --full               binary 설치 후 자동 후속: init + mcp-install + daemon-install
  --help               이 도움말

Environment:
  OPENXGRAM_VERSION       --version 과 동일
  OPENXGRAM_INSTALL_DIR   --install-dir 과 동일

Behavior:
  1. GitHub Releases 의 pre-built tarball 우선 — SHA256 검증 후 설치.
  2. 일치 binary 없거나 download 실패 시 명시적으로 안내 후 cargo fallback.
  3. cargo 미설치 시 raise (silent fallback 금지).
EOF
      exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

# rc.208 — WSL 자동 detect + Windows daemon db symlink (production launch fix)
# Windows daemon (install.ps1) 가 native 로 돌고, WSL CLI 는 그 db 를 공유해야
# cross-machine peer 통신 + peer-per-tmux 가 작동. 다른 data_dir 사용 시
# WSL standalone (peers=0) ↔ Windows daemon db 단절.
detect_wsl() {
  if grep -qi -E "microsoft|wsl" /proc/version 2>/dev/null; then
    return 0  # WSL
  fi
  return 1  # 일반 Linux
}

setup_wsl_symlink() {
  # Windows USERPROFILE 추정 — wslvar USERPROFILE 우선, 없으면 $USER
  local win_user="${USER}"
  if command -v wslvar >/dev/null 2>&1; then
    local win_profile
    win_profile=$(wslvar USERPROFILE 2>/dev/null | tr -d '\r')
    if [ -n "$win_profile" ]; then
      win_user=$(basename "$win_profile")
    fi
  fi
  local win_openxgram="/mnt/c/Users/${win_user}/.openxgram"
  if [ ! -d "$win_openxgram" ]; then
    echo "  → Windows daemon 없음 (path: $win_openxgram). standalone WSL daemon 으로 진행."
    return 1  # symlink skip — fallback 으로 일반 Linux flow
  fi
  # 기존 ~/.openxgram 가 이미 동일 symlink 면 skip
  if [ -L "$HOME/.openxgram" ]; then
    local existing
    existing=$(readlink "$HOME/.openxgram")
    if [ "$existing" = "$win_openxgram" ]; then
      echo "  ✓ 이미 Windows daemon symlink ($HOME/.openxgram → $win_openxgram)"
      return 0
    fi
  fi
  if [ -e "$HOME/.openxgram" ]; then
    local backup="$HOME/.openxgram.bak-$(date +%s)"
    mv "$HOME/.openxgram" "$backup"
    echo "  → 기존 ~/.openxgram → $backup 으로 backup"
  fi
  ln -s "$win_openxgram" "$HOME/.openxgram"
  echo "  ✓ WSL detect: Windows daemon db symlink ($HOME/.openxgram → $win_openxgram)"
  return 0
}

WSL_MODE=0
if detect_wsl; then
  echo "==> WSL 환경 detect"
  if setup_wsl_symlink; then
    WSL_MODE=1
    echo "  → Windows daemon 의 db 공유. WSL standalone daemon 건너뜀."
    echo "  → CLI 만 install (binary download + ~/.local/bin link)."
  fi
fi

case "$OS" in
  linux) OS_ALIAS="linux" ;;
  darwin) OS_ALIAS="darwin" ;;
  msys*|mingw*|cygwin*)
    # Windows 환경 (Git Bash / Cygwin) — POSIX 자동 설치 회피, .zip 안내만 출력 (silent fallback 금지).
    cat <<EOF >&2
[Windows 감지] OS=$OS

이 install.sh 는 POSIX 환경 (Linux/macOS) 자동 설치 전용입니다.
Windows 는 .zip asset 을 직접 다운로드하여 PATH 에 추가해 주세요:

  1) https://github.com/$REPO/releases/latest 접속
  2) xgram-<버전>-x86_64-windows.zip 다운로드
  3) 압축 해제 후 SHA256 검증:
       certutil -hashfile xgram.exe SHA256
       (SHA256SUMS 파일과 비교)
  4) xgram.exe 위치를 시스템 PATH 에 추가

또는 WSL2 (Ubuntu) 에서 이 스크립트를 다시 실행하면 linux 빌드가 자동 설치됩니다.
EOF
    exit 1 ;;
  *) echo "unsupported OS: $OS — build from source: https://github.com/$REPO" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_ALIAS="x86_64" ;;
  aarch64|arm64) ARCH_ALIAS="aarch64" ;;
  *) echo "unsupported arch: $ARCH — build from source: https://github.com/$REPO" >&2; exit 1 ;;
esac

# 지원 매트릭스 (release-binaries.yml 와 동기): asset 이름은 xgram-<tag>-<arch>-<os>.{tar.gz|zip}
#   linux  x86_64   → xgram-<tag>-x86_64-linux.tar.gz
#   linux  aarch64  → xgram-<tag>-aarch64-linux.tar.gz
#   darwin x86_64   → xgram-<tag>-x86_64-darwin.tar.gz
#   darwin aarch64  → xgram-<tag>-aarch64-darwin.tar.gz
#   windows x86_64  → xgram-<tag>-x86_64-windows.zip   (이 install.sh 는 미사용 — 위 Windows 분기에서 안내)
EXPECTED_ASSET_BASENAME="xgram-<tag>-${ARCH_ALIAS}-${OS_ALIAS}.tar.gz"

# 설치 위치 결정
if [ -z "$INSTALL_DIR" ]; then
  if [ -w "${HOME}/.local/bin" ] || mkdir -p "${HOME}/.local/bin" 2>/dev/null; then
    INSTALL_DIR="${HOME}/.local/bin"
  else
    INSTALL_DIR="/usr/local/bin"
  fi
fi

case "$INSTALL_DIR" in
  "${HOME}"*) USE_SUDO="" ;;
  *) USE_SUDO="sudo" ;;
esac

echo "==> OpenXgram installer"
echo "    OS:       $OS_ALIAS"
echo "    Arch:     $ARCH_ALIAS"
echo "    Version:  $VERSION"
echo "    Target:   $INSTALL_DIR/xgram"
echo "    Dry-run:  $DRY_RUN"
echo ""

# ─────────────────────────────────────────────────────────────────────────────
# 1) Pre-built binary download 시도
#    GitHub Releases API: tag → 'latest' 인 경우 가장 최근 release/pre-release 조회.
# ─────────────────────────────────────────────────────────────────────────────

ASSET_NAME=""  # tarball file name (예: xgram-v0.2.0-x86_64-linux.tar.gz)
ASSET_URL=""   # browser_download_url
ASSET_TAG=""   # 실제 사용된 tag

# tag/asset 결정 — curl + grep/sed 만 사용 (jq 의존 회피)
fetch_release_meta() {
  api_url=""
  if [ "$VERSION" = "latest" ]; then
    # latest endpoint 는 pre-release 를 제외하므로 list 에서 직접 첫 항목 사용
    api_url="https://api.github.com/repos/$REPO/releases?per_page=10"
  else
    api_url="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi

  # HSTS / TLS 검증 강제 — `--proto =https --tlsv1.2 --fail`
  meta="$(curl --proto '=https' --tlsv1.2 -fsSL \
            -H 'Accept: application/vnd.github+json' \
            -H 'X-GitHub-Api-Version: 2022-11-28' \
            "$api_url" 2>/dev/null || true)"
  if [ -z "$meta" ]; then
    echo "(meta fetch 실패)" >&2
    return 1
  fi
  printf '%s' "$meta"
}

select_asset_for_target() {
  meta="$1"
  # asset 이름 패턴: xgram-<tag>-<arch>-<os>.tar.gz
  pattern="xgram-.*-${ARCH_ALIAS}-${OS_ALIAS}\\.tar\\.gz"

  # tag_name 첫 매칭
  ASSET_TAG="$(printf '%s' "$meta" \
    | grep -o '"tag_name":[[:space:]]*"[^"]*"' \
    | head -n1 \
    | sed 's/.*"\([^"]*\)"$/\1/')"

  # browser_download_url 중 패턴 매칭하는 것
  ASSET_URL="$(printf '%s' "$meta" \
    | grep -o '"browser_download_url":[[:space:]]*"[^"]*"' \
    | sed 's/.*"\(http[^"]*\)"$/\1/' \
    | grep -E "/$pattern\$" \
    | head -n1 || true)"

  if [ -n "$ASSET_URL" ]; then
    ASSET_NAME="$(basename "$ASSET_URL")"
    return 0
  fi
  return 1
}

PREBUILT_OK="0"
echo "==> Step 1: GitHub Releases 에서 pre-built binary 조회 중..."
echo "    expected asset: $EXPECTED_ASSET_BASENAME"
META="$(fetch_release_meta || true)"
if [ -n "$META" ] && select_asset_for_target "$META"; then
  echo "    found: $ASSET_NAME (tag: $ASSET_TAG)"
  PREBUILT_OK="1"
else
  echo "    pre-built binary 미발견 — ${ARCH_ALIAS}-${OS_ALIAS} 용 asset 없음 또는 release 미공개."
  echo "    (5 타겟 자동 빌드: linux x86_64/aarch64, darwin x86_64/aarch64, windows x86_64)"
  echo "    (silent fallback 금지: 명시적으로 cargo 빌드 경로로 진행합니다)"
fi

if [ "$DRY_RUN" = "1" ]; then
  echo ""
  echo "==> --dry-run: 여기까지 검증 완료. 실제 설치는 하지 않습니다."
  if [ "$PREBUILT_OK" = "1" ]; then
    echo "    pre-built path: OK ($ASSET_URL)"
  else
    echo "    pre-built path: 미해결 → cargo fallback 안내 출력 후 종료했을 것"
  fi
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# 2) Pre-built path 실행 — download + SHA256 검증 + install
# ─────────────────────────────────────────────────────────────────────────────

if [ "$PREBUILT_OK" = "1" ]; then
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  cd "$TMP"

  echo "==> Step 2: tarball 다운로드"
  curl --proto '=https' --tlsv1.2 -fsSL -o "$ASSET_NAME" "$ASSET_URL"

  echo "==> Step 3: SHA256 검증"
  # tarball 안에 SHA256SUMS 가 동봉되어 있음. 풀어서 검증.
  tar -xzf "$ASSET_NAME"
  if [ ! -f xgram ] || [ ! -f SHA256SUMS ]; then
    echo "tarball 구조 이상 (xgram / SHA256SUMS 누락)" >&2
    exit 1
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c SHA256SUMS
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c SHA256SUMS
  else
    echo "sha256sum / shasum 둘 다 미설치 — SHA256 검증 불가." >&2
    echo "이 스크립트는 검증 없이 binary 를 설치하지 않습니다 (보안 정책)." >&2
    exit 1
  fi

  echo "==> Step 4: install → $INSTALL_DIR/xgram"
  chmod +x xgram
  $USE_SUDO mkdir -p "$INSTALL_DIR"
  $USE_SUDO mv xgram "$INSTALL_DIR/xgram"

  # v0.2.0-rc.24~ : Tauri 데스크톱(`xgram-desktop`) 폐기됨 — 웹 GUI 로 전환.
  # release tarball 에도 동봉되지 않음. `xgram gui` 는 Tailscale Funnel URL 을
  # 브라우저로 여는 짧은 launcher 로 동작 (crates/openxgram-cli/src/gui.rs).
  # 옛 binary 가 남아 있을 경우 깨끗하게 제거 (사용자 혼동 방지).
  if [ -f "$INSTALL_DIR/xgram-desktop" ]; then
    $USE_SUDO rm -f "$INSTALL_DIR/xgram-desktop"
    echo "  (옛 xgram-desktop 바이너리 제거 — v0.2.0-rc.24 부터 웹 GUI 로 대체됨)"
  fi

  echo ""
  echo "✓ 설치 완료 (pre-built, tag: $ASSET_TAG) → $INSTALL_DIR/xgram"

  # ──────────────────────────────────────────────────────────────────────────
  # Tailscale 안내 — OpenXgram 이 머신 간 mTLS 메시 전송에 Tailscale 사용.
  # 자동 설치는 Linux 만 (Tailscale 공식 installer). macOS/Windows 는 안내.
  # ──────────────────────────────────────────────────────────────────────────
  echo ""
  if command -v tailscale >/dev/null 2>&1; then
    ts_status="$(tailscale status --peers=false 2>/dev/null | head -1 || true)"
    if echo "$ts_status" | grep -qE "Logged out|stopped"; then
      echo "ℹ Tailscale 설치되어 있으나 로그아웃 상태 — 'tailscale up' 으로 인증"
    else
      echo "✓ Tailscale 발견 — 다른 머신과 메시 통신 가능"
    fi
  else
    case "$OS" in
      linux)
        echo "ℹ Tailscale 미설치 (자율 에이전트 머신간 메시 전송용)."
        echo "   설치:  curl -fsSL https://tailscale.com/install.sh | sh"
        echo "   인증:  sudo tailscale up"
        ;;
      darwin)
        echo "ℹ Tailscale 미설치 (자율 에이전트 머신간 메시 전송용)."
        echo "   설치:  brew install --cask tailscale  ← 또는 App Store"
        echo "   인증:  Tailscale 메뉴바 아이콘 → Log in"
        ;;
      *)
        echo "ℹ Tailscale 미설치 — https://tailscale.com/download 에서 직접 설치"
        ;;
    esac
  fi

  # ──────────────────────────────────────────────────────────────────────────
  # End-to-end setup — Tailscale auto-install + login + xgram init + daemon + pair-desktop.
  # OXG_QUICK=0 으로 끄면 binary 만 설치하고 종료 (예: CI / 컨테이너).
  # 인터랙티브 입력은 /dev/tty 로 직접 — `curl ... | sh` pipe 환경에서도 동작.
  # ──────────────────────────────────────────────────────────────────────────
  if [ "${OXG_QUICK:-1}" = "0" ]; then
    echo ""
    echo "OXG_QUICK=0 — binary 설치만 완료. 'xgram pair-desktop' 등은 수동 실행."
    exit 0
  fi

  if [ ! -e /dev/tty ]; then
    echo ""
    echo "stdin/tty 미접근 — 인터랙티브 wizard 생략. 수동으로 다음을 실행:"
    echo "  sudo tailscale up && xgram daemon & && xgram pair-desktop"
    exit 0
  fi

  echo ""
  echo "==> 자동 설정 시작 (Tailscale + xgram + daemon + pairing)"
  echo "    중단하려면 Ctrl+C — 어느 시점이든 안전 (롤백 가능)."
  echo ""

  # 1. Tailscale 자동 설치 (Linux 만)
  if ! command -v tailscale >/dev/null 2>&1; then
    case "$OS" in
      linux)
        echo "==> Tailscale 설치 (sudo 필요)"
        curl -fsSL https://tailscale.com/install.sh | sh
        ;;
      darwin)
        if command -v brew >/dev/null 2>&1; then
          echo "==> Tailscale 설치 (brew)"
          brew install --cask tailscale
        else
          echo "[중단] macOS — Homebrew 미설치. 다음 중 하나 선택:"
          echo "  - https://brew.sh 에서 brew 설치 후 재실행"
          echo "  - 또는 App Store 에서 Tailscale 설치 + 로그인 후 OXG_QUICK=0 으로 재실행"
          exit 1
        fi
        ;;
    esac
  fi

  # 2. tailscale up 인증
  ts_status="$(tailscale status --peers=false 2>/dev/null | head -1 || true)"
  if echo "$ts_status" | grep -qE "Logged out|stopped" || ! tailscale ip --4 >/dev/null 2>&1; then
    echo "==> Tailscale 로그인"
    echo "    브라우저로 인증 URL 이 열림 — 로그인 후 이 터미널로 돌아오세요."
    sudo tailscale up
  fi
  TS_IP="$(tailscale ip --4 2>/dev/null | head -1)"
  if [ -z "$TS_IP" ]; then
    echo "[중단] tailscale ip --4 출력 비어있음 — 인증 미완료."
    exit 1
  fi
  echo "    Tailscale IP: $TS_IP"

  # 3. xgram init (안 됐을 때만)
  DATA_DIR="${XGRAM_DATA_DIR:-$HOME/.openxgram}"
  if [ ! -f "$DATA_DIR/install-manifest.json" ]; then
    echo ""
    echo "==> xgram init"
    printf "    이 머신 alias (예: gcp-server, macbook): " >/dev/tty
    read -r ALIAS </dev/tty
    if [ -z "$ALIAS" ]; then ALIAS="$(hostname -s 2>/dev/null || echo node)"; fi
    printf "    keystore 패스워드 (12자 이상): " >/dev/tty
    stty -echo </dev/tty 2>/dev/null
    read -r PW1 </dev/tty
    stty echo </dev/tty 2>/dev/null
    printf "\n    패스워드 확인: " >/dev/tty
    stty -echo </dev/tty 2>/dev/null
    read -r PW2 </dev/tty
    stty echo </dev/tty 2>/dev/null
    printf "\n"
    if [ "$PW1" != "$PW2" ]; then
      echo "[중단] 패스워드 불일치"
      exit 1
    fi
    XGRAM_KEYSTORE_PASSWORD="$PW1" xgram init --alias "$ALIAS"
    PW="$PW1"
  else
    echo "    (xgram 이미 초기화됨 — 건너뜀)"
  fi

  # MCP / Claude Code 자동 등록 — 새 LLM 세션이 OpenXgram 도구 + identity + hook 를 즉시 인식.
  # 멱등성 보장 (mcp_install.rs: insert overwrite / inject marker / hook 중복검사).
  if command -v xgram >/dev/null 2>&1; then
    echo ""
    echo "==> Claude Code MCP + CLAUDE.md identity + SessionStart hook 자동 등록"
    xgram mcp-install --scope user --full --use-path-lookup --data-dir "$DATA_DIR" 2>&1 | sed 's/^/    /' || \
      echo "    (mcp-install 실패 — 수동: xgram mcp-install --scope user --full --use-path-lookup)"
  fi

  # rc.117 — ~/oxg.md + 전역 CLAUDE.md @~/oxg.md reference 는 xgram daemon 시작 시
  # 자동 setup (binary embedded). install.sh 가 별도 fetch 안 함.

  # 6.2 — upgrade flow: 옛 daemon/agent 버전 감지 → SIGTERM → 재시작.
  # 메시지 손실 0: SIGTERM 시 daemon 의 graceful shutdown 핸들러가 in-flight envelope 를 commit 후 종료.
  # SQLite WAL 도 fsync 보장.
  CURRENT_VER="$("$INSTALL_DIR/xgram" --version 2>/dev/null | awk '{print $NF}')"
  for proc_kind in daemon agent; do
    PID_LIST="$(pgrep -f "$INSTALL_DIR/xgram $proc_kind" 2>/dev/null || true)"
    if [ -n "$PID_LIST" ]; then
      # 실행 중 binary 의 EXE 경로 비교 (Linux /proc/PID/exe symlink). 차이 있으면 stale.
      RUNNING_PID="$(echo "$PID_LIST" | head -1)"
      RUNNING_EXE="$(readlink "/proc/$RUNNING_PID/exe" 2>/dev/null || true)"
      NEW_EXE="$(readlink -f "$INSTALL_DIR/xgram" 2>/dev/null || echo "$INSTALL_DIR/xgram")"
      if [ -n "$RUNNING_EXE" ] && [ "$RUNNING_EXE" != "$NEW_EXE" ]; then
        echo "==> 옛 $proc_kind 버전 감지 (PID $RUNNING_PID exe=$RUNNING_EXE) → SIGTERM (graceful)"
        kill -TERM "$RUNNING_PID" 2>/dev/null || true
        # 최대 10초 대기 후 강제 종료 (메시지 손실 0 보장: graceful 우선)
        for _ in 1 2 3 4 5 6 7 8 9 10; do
          if ! kill -0 "$RUNNING_PID" 2>/dev/null; then break; fi
          sleep 1
        done
        if kill -0 "$RUNNING_PID" 2>/dev/null; then
          echo "    [경고] $proc_kind PID $RUNNING_PID graceful shutdown 미응답 → SIGKILL"
          kill -KILL "$RUNNING_PID" 2>/dev/null || true
        fi
        echo "    ✓ 옛 $proc_kind 종료 완료 (rc.${CURRENT_VER:-?} 로 재시작 예정)"
      fi
    fi
  done

  # rc.208 — WSL_MODE 면 Windows daemon 의 db 를 share 하므로 WSL 안에서 daemon 안 띄움
  if [ "$WSL_MODE" = "1" ]; then
    echo "    (WSL_MODE: Windows daemon 의 db 공유 — WSL daemon spawn skip)"
  # 4. daemon 백그라운드 가동 (이미 떠 있으면 skip)
  elif ! pgrep -f "$INSTALL_DIR/xgram daemon" >/dev/null 2>&1; then
    echo "==> xgram daemon 가동 (Tailscale IP 에 bind, nohup background)"
    mkdir -p "$DATA_DIR"
    if [ -n "${PW:-}" ]; then
      XGRAM_KEYSTORE_PASSWORD="$PW" nohup "$INSTALL_DIR/xgram" daemon \
        --bind "$TS_IP:47300" --gui-bind "$TS_IP:47302" \
        > "$DATA_DIR/daemon.log" 2>&1 &
    else
      printf "    keystore 패스워드 (daemon 가동용): " >/dev/tty
      stty -echo </dev/tty 2>/dev/null
      read -r PW </dev/tty
      stty echo </dev/tty 2>/dev/null
      printf "\n"
      XGRAM_KEYSTORE_PASSWORD="$PW" nohup "$INSTALL_DIR/xgram" daemon \
        --bind "$TS_IP:47300" --gui-bind "$TS_IP:47302" \
        > "$DATA_DIR/daemon.log" 2>&1 &
    fi
    sleep 2
    if ! pgrep -f "$INSTALL_DIR/xgram daemon" >/dev/null 2>&1; then
      echo "[중단] daemon 시작 실패. log 확인: $DATA_DIR/daemon.log"
      exit 1
    fi
    echo "    ✓ daemon 가동 (log: $DATA_DIR/daemon.log)"
  else
    echo "    (daemon 이미 가동 중 — 건너뜀)"
  fi

  # 4b. agent 런타임 가동 (inbox 폴링 + Discord 양방향)
  #     - XGRAM_DISCORD_WEBHOOK_URL: outbound (inbox → Discord forward)
  #     - XGRAM_DISCORD_BOT_TOKEN + XGRAM_DISCORD_CHANNEL_ID: inbound (Discord → daemon)
  # rc.208 — WSL_MODE 면 Windows agent 가 이미 돌고 있으므로 WSL agent spawn skip.
  if [ "$WSL_MODE" = "1" ]; then
    echo "    (WSL_MODE: Windows agent 가 inbox 처리 — WSL agent spawn skip)"
  elif ! pgrep -f "$INSTALL_DIR/xgram agent" >/dev/null 2>&1; then
    echo "==> xgram agent 런타임 가동 (inbox 폴링 + Discord 양방향)"
    XGRAM_DISCORD_WEBHOOK_URL="${XGRAM_DISCORD_WEBHOOK_URL:-}" \
    XGRAM_DISCORD_BOT_TOKEN="${XGRAM_DISCORD_BOT_TOKEN:-}" \
    XGRAM_DISCORD_CHANNEL_ID="${XGRAM_DISCORD_CHANNEL_ID:-}" \
    XGRAM_ANTHROPIC_API_KEY="${XGRAM_ANTHROPIC_API_KEY:-}" \
    XGRAM_TELEGRAM_BOT_TOKEN="${XGRAM_TELEGRAM_BOT_TOKEN:-}" \
    XGRAM_TELEGRAM_CHAT_ID="${XGRAM_TELEGRAM_CHAT_ID:-}" \
      nohup "$INSTALL_DIR/xgram" agent \
      > "$DATA_DIR/agent.log" 2>&1 &
    echo "    Discord webhook : ${XGRAM_DISCORD_WEBHOOK_URL:+설정됨}${XGRAM_DISCORD_WEBHOOK_URL:-미설정}"
    echo "    Discord inbound : ${XGRAM_DISCORD_BOT_TOKEN:+bot 토큰 설정됨}${XGRAM_DISCORD_BOT_TOKEN:-bot 토큰 미설정}"
    echo "    Telegram bot    : ${XGRAM_TELEGRAM_BOT_TOKEN:+설정됨}${XGRAM_TELEGRAM_BOT_TOKEN:-미설정}"
    echo "    Anthropic LLM   : ${XGRAM_ANTHROPIC_API_KEY:+활성 (claude-haiku 4.5)}${XGRAM_ANTHROPIC_API_KEY:-비활성 (echo 응답)}"
    sleep 1
    if ! pgrep -f "$INSTALL_DIR/xgram agent" >/dev/null 2>&1; then
      echo "    [경고] agent 시작 실패. log 확인: $DATA_DIR/agent.log"
    else
      echo "    ✓ agent 가동 (log: $DATA_DIR/agent.log)"
    fi
  else
    echo "    (agent 이미 가동 중 — 건너뜀)"
  fi

  # 5. pair-desktop URL 출력
  echo ""
  echo "==> 페어링 URL 발급"
  PAIRING_OUTPUT="$("$INSTALL_DIR/xgram" pair-desktop 2>&1)"
  echo "$PAIRING_OUTPUT"

  # 6. Tailscale Funnel 자동 활성화 — 외부 https 노출.
  #    daemon 이 ${TS_IP}:47302 에 GUI 직접 서빙 (rc.26 정적 자산 임베드).
  #    Funnel target 은 같은 IP:47302 (localhost 가 아님 — daemon 이 TS_IP 에 bind).
  echo ""
  echo "==> Web GUI 외부 노출 (Tailscale Funnel)"
  GUI_PORT=47302
  # daemon 이 어디 listen 하나 확인 — Tailscale IP 우선, fallback 으로 127.0.0.1.
  FUNNEL_TARGET="http://${TS_IP}:${GUI_PORT}"
  if ! curl -s -o /dev/null --max-time 2 "$FUNNEL_TARGET/gui/" 2>/dev/null; then
    if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:${GUI_PORT}/gui/" 2>/dev/null; then
      FUNNEL_TARGET="http://127.0.0.1:${GUI_PORT}"
    fi
  fi
  echo "    daemon GUI : ${FUNNEL_TARGET}"
  # 옛 funnel (다른 port) 가 켜진 상태면 reset 후 새로 — funnel 은 한 https port 당
  # 하나의 target 만 가질 수 있어서 충돌 회피.
  if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    sudo tailscale funnel reset 2>/dev/null || true
  fi
  # Hostname 추출 — 3 단계 fallback:
  #   (1) tailscale status --json 의 Self.DNSName (trailing dot 제거)
  #   (2) 페어링 URL 안의 hostname (pair-desktop 출력 reuse)
  #   (3) tailscale status 텍스트 첫 줄의 short hostname + MagicDNS suffix
  # python3/jq 의존 X — grep/sed 만 사용.
  TS_NAME=""
  if command -v tailscale >/dev/null 2>&1; then
    # tailscale status --json 의 첫 "DNSName":"<host>." 추출. trailing dot 제거.
    TS_NAME="$(tailscale status --json 2>/dev/null \
      | grep -o '"DNSName":[[:space:]]*"[^"]*"' \
      | head -1 \
      | sed 's/.*"\([^"]*\)"$/\1/; s/\.$//')"
  fi
  # fallback — PAIRING_OUTPUT (xgram pair-desktop) 안의 oxg:// URL 에서 hostname 추출.
  if [ -z "$TS_NAME" ]; then
    TS_NAME="$(printf '%s' "$PAIRING_OUTPUT" | grep -oE '[a-zA-Z0-9-]+\.tail[a-z0-9]+\.ts\.net' | head -1)"
  fi

  if [ -n "$TS_NAME" ]; then
    echo "    감지된 hostname: ${TS_NAME}"
    FUNNEL_DONE=0
    if command -v sudo >/dev/null 2>&1; then
      # 1단계 — NOPASSWD 가능하면 즉시 자동 활성화 (CI/스크립트 환경)
      if sudo -n true 2>/dev/null; then
        FUNNEL_OUT="$(sudo tailscale funnel --bg --https=443 "$FUNNEL_TARGET" 2>&1)"
        if echo "$FUNNEL_OUT" | grep -qE "Available|on the internet|Funnel started"; then
          echo "    ✓ Funnel 활성화 완료 (sudo NOPASSWD)"
          FUNNEL_DONE=1
        fi
      fi
      # 2단계 — /dev/tty 로 사용자 비밀번호 prompt + 응답.
      # stderr (sudo prompt) 도 /dev/tty 로 보내야 사용자에게 보임 (pipe 안에 묻히지 않게).
      if [ "$FUNNEL_DONE" = "0" ] && [ -e /dev/tty ]; then
        echo "    sudo 비밀번호 한 번 입력 (Funnel 활성화 — 한 번만)"
        sudo -p "  [sudo] password: " \
             tailscale funnel --bg --https=443 "$FUNNEL_TARGET" \
             </dev/tty 2>/dev/tty > /tmp/xgram-funnel.out
        if grep -qE "Available|on the internet|Funnel started" /tmp/xgram-funnel.out 2>/dev/null; then
          echo "    ✓ Funnel 활성화 완료"
          FUNNEL_DONE=1
        else
          echo "    ⚠ Funnel 활성화 실패 — 수동 실행:"
          echo "       sudo tailscale funnel --bg --https=443 $FUNNEL_TARGET"
        fi
      fi
      if [ "$FUNNEL_DONE" = "0" ]; then
        echo "    (sudo 권한 또는 TTY 없음 — 다음 한 줄 직접 실행)"
        echo "       sudo tailscale funnel --bg --https=443 $FUNNEL_TARGET"
      fi
    else
      echo "    (sudo 없음 — 다음 한 줄 직접 실행)"
      echo "       tailscale funnel --bg --https=443 $FUNNEL_TARGET"
    fi
    # 활성화 여부와 무관하게 GUI URL 안내.
    echo ""
    echo "  ┌─ Web GUI 접속 ─────────────────────────────────────────────┐"
    echo "  │  https://${TS_NAME}/gui/"
    echo "  │  비밀번호 = xgram init 때 정한 keystore 비밀번호"
    echo "  └────────────────────────────────────────────────────────────┘"
  else
    echo "    ⚠ Tailscale hostname 추출 실패 — 수동 확인:"
    echo "       tailscale status --json | jq -r .Self.DNSName"
    echo "       sudo tailscale funnel --bg --https=443 $FUNNEL_TARGET"
  fi

  echo ""
  echo "✓ 설치 완료. 데스크탑/노트북에서 이 daemon 에 attach 하려면:"
  echo "    curl -sSfL https://openxgram.org/install.sh | sh"
  echo "    xgram link '<oxg URL>'  # 위 페어링 URL"
  echo ""
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# 3) Cargo fallback — pre-built 미발견 시 명시적으로 안내 후 진행
# ─────────────────────────────────────────────────────────────────────────────

echo ""
echo "==> Cargo fallback: 소스에서 빌드합니다."
echo "    (pre-built ${ARCH_ALIAS}-${OS_ALIAS} binary 가 release 에 없으므로 fallback)"
echo ""

if ! command -v cargo >/dev/null 2>&1; then
  cat <<EOF
[중단] cargo 가 설치되어 있지 않습니다.

선택지:
  A) Rust 설치 후 다시 시도:
       curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
       curl -sSfL https://openxgram.org/install.sh | sh
  B) 다른 머신에서 build → ssh/scp 로 binary 전송
  C) https://github.com/$REPO/releases 에서 자신의 OS/arch 용 asset 요청

EOF
  exit 1
fi

echo "==> cargo 발견. 소스에서 build (5~10분 소요)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cd "$TMP"
git clone --depth 1 --branch "main" "https://github.com/$REPO" openxgram
cd openxgram

# 특정 버전 태그 지정 시 checkout
if [ "$VERSION" != "latest" ] && [ "$VERSION" != "main" ]; then
  git fetch --depth 1 origin "refs/tags/$VERSION:refs/tags/$VERSION" || git fetch --depth 1 origin "$VERSION"
  git checkout FETCH_HEAD
fi

cargo install --path crates/openxgram-cli --locked --root "$(dirname "$INSTALL_DIR")"

echo ""
echo "✓ 설치 완료 (cargo fallback) → $INSTALL_DIR/xgram"
echo ""
echo "다음 단계:"
echo "  xgram --version"
echo "  xgram init --alias <name>"
echo "  xgram mcp-install --scope user --full --use-path-lookup    # Claude Code MCP + identity + hook"
echo ""

# --full: 후속 자동 실행 (init + mcp-install + daemon-install)
# /dev/tty 없는 환경 (systemd 등) 자동 감지 — interactive prompt 회피.
if [ "$FULL" = "1" ] && [ "$DRY_RUN" = "0" ]; then
  XGRAM_BIN="$INSTALL_DIR/xgram"
  if [ ! -x "$XGRAM_BIN" ]; then XGRAM_BIN="$(command -v xgram || true)"; fi
  if [ -z "$XGRAM_BIN" ]; then
    echo "⚠ --full: xgram binary 못 찾음. 위 install 단계 확인 후 수동 실행."
    exit 0
  fi
  echo "==> --full: 후속 자동 설치"
  # interactive 환경에서만 init prompt (TTY 없으면 alias 자동 = hostname)
  if [ -t 0 ]; then
    "$XGRAM_BIN" init || true
  else
    "$XGRAM_BIN" init --alias "$(hostname -s 2>/dev/null || echo node)" || true
  fi
  "$XGRAM_BIN" mcp-install --scope user --full --use-path-lookup || true
  # rc.208 — WSL_MODE 면 daemon-install (systemd user service) skip — Windows daemon 공유
  if [ "$WSL_MODE" = "1" ]; then
    echo "  (WSL_MODE: daemon-install skip — Windows daemon 의 db 공유)"
  else
    "$XGRAM_BIN" daemon-install || true
    echo ""
    echo "✓ --full 완료. systemd user 활성화:"
    echo "  systemctl --user enable --now openxgram-sidecar.service"
  fi
fi
