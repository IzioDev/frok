#!/usr/bin/env sh
set -e

BASE_URL="https://github.com/IzioDev/frok/releases"

if [ -n "$FROK_VERSION" ]; then
  DOWNLOAD_BASE="$BASE_URL/download/v$FROK_VERSION"
else
  DOWNLOAD_BASE="$BASE_URL/latest/download"
fi

OS="$(uname -s)"
case "$OS" in
  Linux) OS="linux" ;;
  Darwin) OS="darwin" ;;
  *) echo "unsupported OS: $OS" >&2; exit 1 ;;
esac

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

ARCHIVE="frok-$OS-$ARCH.tar.gz"
TMP_DIR="$(mktemp -d)"

curl -fsSL "$DOWNLOAD_BASE/$ARCHIVE" -o "$TMP_DIR/$ARCHIVE"
tar -xzf "$TMP_DIR/$ARCHIVE" -C "$TMP_DIR"

if [ -n "$FROK_INSTALL_DIR" ]; then
  BIN_DIR="$FROK_INSTALL_DIR"
else
  BIN_DIR="$HOME/.local/bin"
fi

mkdir -p "$BIN_DIR"
mv "$TMP_DIR/frok" "$BIN_DIR/frok"
chmod +x "$BIN_DIR/frok"

echo "installed: $BIN_DIR/frok"
if command -v frok >/dev/null 2>&1; then
  frok --version || true
else
  echo "add to PATH: $BIN_DIR"
fi
