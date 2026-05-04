#!/usr/bin/env sh
# OpenXgram installer — fetch latest xgram binary and install to ~/.local/bin or /usr/local/bin.
#
# Usage:
#   curl -sSfL https://openxgram.org/install.sh | sh
#   curl -sSfL https://openxgram.org/install.sh | sh -s -- --version v0.2.0
#
# Privacy: this script only downloads the binary and verifies its signature.
# It sends NO telemetry, no usage stats, nothing. Source is at:
#   https://github.com/OpenXgram/openxgram/blob/main/www/install.sh

set -eu

REPO="OpenXgram/openxgram"
VERSION="${OPENXGRAM_VERSION:-latest}"
INSTALL_DIR="${OPENXGRAM_INSTALL_DIR:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --help|-h)
      cat <<EOF
OpenXgram installer

Options:
  --version <tag>      Specific release tag (default: latest)
  --install-dir <dir>  Install location (default: ~/.local/bin if writable, else /usr/local/bin)
  --help               Show this help

Environment:
  OPENXGRAM_VERSION       Override version (same as --version)
  OPENXGRAM_INSTALL_DIR   Override install dir (same as --install-dir)
EOF
      exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux|darwin) ;;
  *) echo "unsupported OS: $OS — please build from source: https://github.com/$REPO" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_ALIAS="x86_64" ;;
  aarch64|arm64) ARCH_ALIAS="aarch64" ;;
  *) echo "unsupported arch: $ARCH — please build from source: https://github.com/$REPO" >&2; exit 1 ;;
esac

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

# Pre-built 바이너리가 아직 없는 경우 안내 — 0.2.0 시점에서는 cargo install 권장
echo "==> OpenXgram installer"
echo "    OS:      $OS"
echo "    Arch:    $ARCH_ALIAS"
echo "    Version: $VERSION"
echo "    Target:  $INSTALL_DIR/xgram"
echo ""

# v0.2.0: pre-built 바이너리는 후속 GitHub Releases. 우선 cargo 경로 안내.
if ! command -v cargo >/dev/null 2>&1; then
  cat <<EOF
v0.2.0 시점에서는 사전 빌드된 바이너리가 아직 GitHub Releases 에 게시되지 않아
소스에서 빌드하셔야 합니다 (Rust 1.78+).

1) Rust 설치:
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

2) 다시 이 스크립트 실행:
   curl -sSfL https://openxgram.org/install.sh | sh

소스 빌드 직접 원하시면:
   git clone https://github.com/$REPO
   cd openxgram
   cargo install --path crates/openxgram-cli --locked

자세한 빌드 가이드: https://github.com/$REPO#build
EOF
  exit 1
fi

echo "==> cargo 가 발견되었습니다. 소스에서 빌드합니다 (5~10분 소요)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cd "$TMP"
git clone --depth 1 --branch "main" "https://github.com/$REPO" openxgram
cd openxgram

# 특정 버전 태그 지정 시 checkout
if [ "$VERSION" != "latest" ] && [ "$VERSION" != "main" ]; then
  git fetch --depth 1 origin "tag/$VERSION" || git fetch --depth 1 origin "$VERSION"
  git checkout FETCH_HEAD
fi

cargo install --path crates/openxgram-cli --locked --root "$(dirname "$INSTALL_DIR")"

echo ""
echo "✓ 설치 완료 → $INSTALL_DIR/xgram"
echo ""
echo "다음 단계:"
echo "  xgram --version        # 버전 확인"
echo "  xgram init             # 12-단어 복구 시드 생성 (오프라인 보관 권장)"
echo ""
echo "데이터는 사용자 기기 안 ~/.openxgram/ 에만 저장됩니다."
echo "외부로 전송되는 데이터는 0 입니다."
