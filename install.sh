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

# A specific version, not a moving "latest": an install that silently changes what it
# fetches cannot be reproduced, and a pinned version is what lets a lockfile or a
# Dockerfile mean anything. Override with NEOBROWSER_VERSION.
VERSION="${NEOBROWSER_VERSION:-}"
if [ -z "$VERSION" ]; then
  echo "Resolving the latest release tag..."
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
fi
if [ -z "$VERSION" ]; then
  echo "could not resolve a release version; set NEOBROWSER_VERSION=vX.Y.Z" >&2
  exit 1
fi
echo "Installing ${VERSION}"

art="neobrowser-${target}.tar.gz"
base="https://github.com/${REPO}/releases/download/${VERSION}"
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
# Build provenance, when the GitHub CLI is available. Releases are attested by
# release.yml, so this proves the artifact was built by that workflow from this repo —
# a checksum only proves the file matches a checksum published beside it, which the
# same attacker would control.
if command -v gh >/dev/null 2>&1; then
  echo "Verifying build provenance..."
  if gh attestation verify "$tmp/$art" --repo "${REPO}" >/dev/null 2>&1; then
    echo "Provenance: verified (built by ${REPO} release workflow)"
  else
    echo "warning: provenance could not be verified. Continuing, since gh may not be" >&2
    echo "         authenticated; run 'gh attestation verify' yourself to be certain." >&2
  fi
else
  echo "note: install the GitHub CLI to verify build provenance (gh attestation verify)"
fi

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
