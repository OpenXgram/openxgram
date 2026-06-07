#!/usr/bin/env sh
# OpenXgram installer (repo-root canonical) — pre-built binary 우선, 실패 시 cargo 빌드 fallback.
#
# 이 파일은 `curl -sSfL https://openxgram.org/install.sh | sh` 와
# `curl -sSfL https://raw.githubusercontent.com/OpenXgram/openxgram/main/install.sh | sh`
# 양쪽에서 동작하는 정본 installer 다. (www/public/install.sh 는 사이트 배포 사본)
#
# Usage:
#   curl -sSfL https://openxgram.org/install.sh | sh
#   curl -sSfL https://openxgram.org/install.sh | sh -s -- --tag v0.2.0-rc.268
#   curl -sSfL https://openxgram.org/install.sh | sh -s -- --dry-run
#
# 실사용 3 플랫폼만 pre-built 자산 게시 (release-binaries.yml 와 동기):
#   x86_64-linux   → xgram-<tag>-x86_64-linux.tar.gz
#   aarch64-darwin → xgram-<tag>-aarch64-darwin.tar.gz   (Apple Silicon)
#   x86_64-windows → xgram-<tag>-x86_64-windows.zip      (install.ps1 사용 권장)
# (intel-mac / arm-linux 는 현재 제외 — cargo fallback 안내)
#
# Privacy: GitHub Releases asset 만 download + SHA256 검증 후 install. 텔레메트리 0.
# Source: https://github.com/OpenXgram/openxgram/blob/main/install.sh

set -eu

REPO="OpenXgram/openxgram"
VERSION="${OPENXGRAM_VERSION:-latest}"
INSTALL_DIR="${OPENXGRAM_INSTALL_DIR:-}"
DRY_RUN="0"

while [ $# -gt 0 ]; do
  case "$1" in
    --version|--tag) VERSION="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --dry-run) DRY_RUN="1"; shift 1 ;;
    --help|-h)
      cat <<EOF
OpenXgram installer

Options:
  --tag <tag>          특정 release tag (default: latest pre-release/release)
  --version <tag>      --tag 와 동일
  --install-dir <dir>  설치 위치 (default: ~/.local/bin)
  --dry-run            검증만 — 실제 설치는 하지 않음
  --help               이 도움말

Environment:
  OPENXGRAM_VERSION       --tag 과 동일
  OPENXGRAM_INSTALL_DIR   --install-dir 과 동일

지원 플랫폼 (pre-built):
  x86_64-linux, aarch64-darwin (Apple Silicon), x86_64-windows
EOF
      exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux) OS_ALIAS="linux" ;;
  darwin) OS_ALIAS="darwin" ;;
  msys*|mingw*|cygwin*)
    cat <<EOF >&2
[Windows 감지] OS=$OS

이 install.sh 는 POSIX 환경 (Linux/macOS) 자동 설치 전용입니다.
Windows 는 PowerShell 설치를 사용하세요:

  irm https://openxgram.org/install.ps1 | iex

또는 .zip 자산을 직접 다운로드:
  1) https://github.com/$REPO/releases/latest 접속
  2) xgram-<버전>-x86_64-windows.zip 다운로드
  3) 압축 해제 후 SHA256 검증: certutil -hashfile xgram.exe SHA256
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

# 실사용 3 플랫폼 매트릭스 (release-binaries.yml allow_failure:false 레그와 동기):
#   x86_64-linux  / aarch64-darwin / x86_64-windows
# 제외 플랫폼(현재 pre-built 없음): x86_64-darwin (intel-mac), aarch64-linux (arm-linux)
case "${ARCH_ALIAS}-${OS_ALIAS}" in
  x86_64-linux|aarch64-darwin)
    PREBUILT_SUPPORTED=1 ;;
  x86_64-windows)
    PREBUILT_SUPPORTED=1 ;;  # 위 Windows 분기에서 이미 처리됨 (도달 안 함)
  *)
    PREBUILT_SUPPORTED=0 ;;  # x86_64-darwin / aarch64-linux → cargo fallback
esac

EXPECTED_ASSET_BASENAME="xgram-<tag>-${ARCH_ALIAS}-${OS_ALIAS}.tar.gz"

# 설치 위치 결정 (default: ~/.local/bin)
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

if [ "$PREBUILT_SUPPORTED" = "0" ]; then
  cat <<EOF >&2
ℹ pre-built binary 미제공 플랫폼: ${ARCH_ALIAS}-${OS_ALIAS}

현재 자동 게시되는 pre-built 플랫폼은 3 종입니다:
  - x86_64-linux
  - aarch64-darwin (Apple Silicon)
  - x86_64-windows  (irm https://openxgram.org/install.ps1 | iex)

(intel-mac / arm-linux 는 추후 복구 예정)

→ 소스에서 빌드 (cargo) 경로로 진행합니다.
EOF
fi

# ─────────────────────────────────────────────────────────────────────────────
# 1) Pre-built binary 조회 (GitHub Releases API) — jq 의존 없이 curl + grep/sed.
# ─────────────────────────────────────────────────────────────────────────────

ASSET_NAME=""
ASSET_URL=""
ASSET_TAG=""

fetch_release_meta() {
  api_url=""
  if [ "$VERSION" = "latest" ]; then
    # latest endpoint 는 pre-release 제외 → list 에서 첫 항목 사용 (rc.* 게시 중)
    api_url="https://api.github.com/repos/$REPO/releases?per_page=10"
  else
    api_url="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi
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
  pattern="xgram-.*-${ARCH_ALIAS}-${OS_ALIAS}\\.tar\\.gz"
  ASSET_TAG="$(printf '%s' "$meta" \
    | grep -o '"tag_name":[[:space:]]*"[^"]*"' \
    | head -n1 \
    | sed 's/.*"\([^"]*\)"$/\1/')"
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
if [ "$PREBUILT_SUPPORTED" = "1" ]; then
  echo "==> Step 1: GitHub Releases 에서 pre-built binary 조회 중..."
  echo "    expected asset: $EXPECTED_ASSET_BASENAME"
  META="$(fetch_release_meta || true)"
  if [ -n "$META" ] && select_asset_for_target "$META"; then
    echo "    found: $ASSET_NAME (tag: $ASSET_TAG)"
    PREBUILT_OK="1"
  else
    echo "    pre-built binary 미발견 — ${ARCH_ALIAS}-${OS_ALIAS} 자산 없음 또는 release 미공개."
    echo "    (silent fallback 금지: 명시적으로 cargo 빌드 경로로 진행합니다)"
  fi
fi

if [ "$DRY_RUN" = "1" ]; then
  echo ""
  echo "==> --dry-run: 여기까지 검증 완료. 실제 설치는 하지 않습니다."
  if [ "$PREBUILT_OK" = "1" ]; then
    echo "    pre-built path: OK ($ASSET_URL)"
  else
    echo "    pre-built path: 미해결 → cargo fallback 경로"
  fi
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# 2) Pre-built path — download + SHA256 검증 + install → ~/.local/bin/xgram
# ─────────────────────────────────────────────────────────────────────────────

if [ "$PREBUILT_OK" = "1" ]; then
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  cd "$TMP"

  echo "==> Step 2: tarball 다운로드"
  curl --proto '=https' --tlsv1.2 -fsSL -o "$ASSET_NAME" "$ASSET_URL"

  echo "==> Step 3: SHA256 검증"
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

  echo ""
  echo "✓ 설치 완료 (pre-built, tag: $ASSET_TAG) → $INSTALL_DIR/xgram"
  echo ""
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) : ;;
    *) echo "ℹ PATH 에 $INSTALL_DIR 추가 필요:"
       echo "    export PATH=\"$INSTALL_DIR:\$PATH\"   # ~/.bashrc 또는 ~/.zshrc 에 추가" ;;
  esac

  echo ""
  echo "다음 단계 (데몬·MCP·페어링 자동 설정):"
  echo "  xgram --version"
  echo "  xgram init --alias <name>"
  echo "  xgram mcp-install --scope user --full --use-path-lookup   # Claude Code MCP + identity + hook"
  echo "  xgram daemon-install                                       # systemd/launchd 영속 데몬 (이미 구현됨)"
  echo ""
  echo "전체 자동 설정(Tailscale + daemon + Funnel)을 원하면 사이트 installer 사용:"
  echo "  curl -sSfL https://openxgram.org/install.sh | sh"
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# 3) Cargo fallback — pre-built 미발견 시 명시적 안내 후 진행 (silent 금지)
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
echo "  xgram mcp-install --scope user --full --use-path-lookup"
echo "  xgram daemon-install"
echo ""
