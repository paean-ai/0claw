#!/usr/bin/env bash
set -eu

##############################################################################
# 0claw Install Script
#
# Downloads the latest 0claw binary from GitHub releases.
# Re-run to update.
#
# Usage:
#   curl -fsSL https://0.works/install.sh | bash
#
# Environment variables:
#   ZEROCLAW_BIN_DIR  - Install directory (default: $HOME/.local/bin)
#   ZEROCLAW_VERSION  - Specific version, e.g. "v0.1.0" (default: stable)
##############################################################################

REPO="paean-ai/0claw"
BIN="0claw"

if ! command -v curl >/dev/null 2>&1; then
  echo "Error: curl is required." >&2
  exit 1
fi

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux*)  os="unknown-linux-gnu" ;;
    Darwin*) os="apple-darwin" ;;
    MINGW*|MSYS*|CYGWIN*) os="pc-windows-msvc" ;;
    *) echo "Unsupported OS: $os" >&2; exit 1 ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
  esac

  echo "${arch}-${os}"
}

PLATFORM="$(detect_platform)"
BIN_DIR="${ZEROCLAW_BIN_DIR:-$HOME/.local/bin}"
TAG="${ZEROCLAW_VERSION:-stable}"

if [ "$TAG" = "stable" ]; then
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/stable/0claw-${PLATFORM}.tar.gz"
else
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${TAG}/0claw-${PLATFORM}.tar.gz"
fi

echo "  0claw installer"
echo "  Platform: ${PLATFORM}"
echo "  Version:  ${TAG}"
echo ""

mkdir -p "$BIN_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "  Downloading ${DOWNLOAD_URL}..."
HTTP_CODE=$(curl -fsSL -w "%{http_code}" -o "$TMP/0claw.tar.gz" "$DOWNLOAD_URL" 2>/dev/null) || true

if [ ! -f "$TMP/0claw.tar.gz" ] || [ "$HTTP_CODE" != "200" ]; then
  echo ""
  echo "  Binary not available for ${PLATFORM} (${TAG})."
  echo "  Falling back to cargo install..."
  echo ""
  if command -v cargo >/dev/null 2>&1; then
    cargo install zeroclaw
    echo ""
    echo "  Installed via cargo. Run: 0claw"
    exit 0
  else
    echo "  Error: cargo not found. Install Rust first: https://rustup.rs" >&2
    exit 1
  fi
fi

tar -xzf "$TMP/0claw.tar.gz" -C "$TMP"

if [ -f "$TMP/0claw" ]; then
  mv "$TMP/0claw" "$BIN_DIR/$BIN"
elif [ -f "$TMP/$BIN" ]; then
  mv "$TMP/$BIN" "$BIN_DIR/$BIN"
else
  BIN_FILE=$(find "$TMP" -name "0claw" -type f | head -1)
  if [ -n "$BIN_FILE" ]; then
    mv "$BIN_FILE" "$BIN_DIR/$BIN"
  else
    echo "  Error: binary not found in archive" >&2
    exit 1
  fi
fi

chmod +x "$BIN_DIR/$BIN"

echo "  Installed to ${BIN_DIR}/${BIN}"

if ! echo "$PATH" | tr ':' '\n' | grep -q "^${BIN_DIR}$"; then
  echo ""
  echo "  Add to your PATH:"
  echo "    export PATH=\"${BIN_DIR}:\$PATH\""
  echo ""
  SHELL_NAME="$(basename "$SHELL" 2>/dev/null || echo "bash")"
  case "$SHELL_NAME" in
    zsh)  RC="$HOME/.zshrc" ;;
    bash) RC="$HOME/.bashrc" ;;
    fish) RC="$HOME/.config/fish/config.fish" ;;
    *)    RC="$HOME/.profile" ;;
  esac
  echo "  Or add it permanently:"
  echo "    echo 'export PATH=\"${BIN_DIR}:\$PATH\"' >> ${RC}"
fi

echo ""
echo "  Done. Run: 0claw"
