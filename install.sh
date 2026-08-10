#!/bin/sh
# NeoBrowser installer — downloads the latest release binary for your platform.
#
#   curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh
#
# Override the install dir with NEOBROWSER_INSTALL_DIR (default: /usr/local/bin).
set -eu

REPO="pitiflautico/neobrowser"
INSTALL_DIR="${NEOBROWSER_INSTALL_DIR:-/usr/local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) case "$arch" in
            arm64) target="aarch64-apple-darwin" ;;
            x86_64) target="x86_64-apple-darwin" ;;
            *) echo "unsupported macOS arch: $arch" >&2; exit 1 ;;
          esac ;;
  Linux)  case "$arch" in
            x86_64) target="x86_64-unknown-linux-gnu" ;;
            *) echo "unsupported Linux arch: $arch (build from source: cargo build --release)" >&2; exit 1 ;;
          esac ;;
  *) echo "unsupported OS: $os (on Windows, download the .zip from the Releases page)" >&2; exit 1 ;;
esac

url="https://github.com/${REPO}/releases/latest/download/neobrowser-${target}.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading neobrowser (${target})..."
curl -fsSL "$url" -o "$tmp/neobrowser.tar.gz"
tar -C "$tmp" -xzf "$tmp/neobrowser.tar.gz"
chmod +x "$tmp/neobrowser"

if [ -w "$INSTALL_DIR" ]; then
  mv "$tmp/neobrowser" "$INSTALL_DIR/neobrowser"
else
  echo "Installing to $INSTALL_DIR (needs sudo)..."
  sudo mv "$tmp/neobrowser" "$INSTALL_DIR/neobrowser"
fi

echo "Installed: $(command -v neobrowser)"
echo "Next: run 'neobrowser doctor' to verify Chrome is found."
