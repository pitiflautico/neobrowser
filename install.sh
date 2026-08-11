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

art="neobrowser-${target}.tar.gz"
base="https://github.com/${REPO}/releases/latest/download"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading neobrowser (${target})..."
curl -fsSL "$base/$art" -o "$tmp/$art"
curl -fsSL "$base/$art.sha256" -o "$tmp/$art.sha256" || echo "warning: checksum file unavailable"

if [ -s "$tmp/$art.sha256" ]; then
  echo "Verifying checksum..."
  if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$tmp" && sha256sum -c "$art.sha256" ) || { echo "checksum FAILED — aborting" >&2; exit 1; }
  elif command -v shasum >/dev/null 2>&1; then
    ( cd "$tmp" && shasum -a 256 -c "$art.sha256" ) || { echo "checksum FAILED — aborting" >&2; exit 1; }
  else
    echo "warning: no sha256 tool found; skipping verification"
  fi
fi
# Stronger (optional): verify signed build provenance with the GitHub CLI:
#   gh attestation verify "$tmp/$art" --repo ${REPO}

tar -C "$tmp" -xzf "$tmp/$art"
chmod +x "$tmp/neobrowser"

if [ -w "$INSTALL_DIR" ]; then
  mv "$tmp/neobrowser" "$INSTALL_DIR/neobrowser"
else
  echo "Installing to $INSTALL_DIR (needs sudo)..."
  sudo mv "$tmp/neobrowser" "$INSTALL_DIR/neobrowser"
fi

echo "Installed: $(command -v neobrowser)"
echo "Next: run 'neobrowser doctor' to verify Chrome is found."
