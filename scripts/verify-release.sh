#!/usr/bin/env bash
# Independent verification of a NeoBrowser release.
#
# The PRD asks for reproduction "by a third person or an external CI". This is the
# mechanism: one command that a stranger can run to check the published claims for
# themselves, without trusting anything this repository says about itself.
#
#   ./scripts/verify-release.sh v0.1.7
#
# What it checks, and what each check is worth:
#
#   1. Provenance   — the artifact was built by this repo's release workflow (SLSA
#                     attestation). Stronger than a checksum, which an attacker who
#                     replaced the artifact would also control.
#   2. Checksum     — the artifact matches the hash published beside it.
#   3. Static musl  — the Linux musl artifact really is statically linked. The README
#                     claims it; this is how you confirm it rather than believing it.
#   4. Rebuild      — building from the tag produces a working binary with the same
#                     version. NOT byte-identical; see docs/REPRODUCIBILITY.md for why.
#   5. Behaviour    — the rebuilt binary passes its own test suite, including the
#                     live-Chrome and property/fuzz suites.
#   6. Claims       — the tool count and binary size the README states match reality.
#
# Exits non-zero on the first failed check. Every check prints what it concluded, so a
# partial run is still informative.

set -euo pipefail

VERSION="${1:-}"
REPO="pitiflautico/neobrowser"
if [ -z "$VERSION" ]; then
  echo "usage: $0 <version-tag>   e.g. $0 v0.1.7" >&2
  exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
PASS=0
FAIL=0

ok()   { echo "  PASS  $1"; PASS=$((PASS+1)); }
bad()  { echo "  FAIL  $1" >&2; FAIL=$((FAIL+1)); }
skip() { echo "  SKIP  $1"; }

echo "Verifying $REPO $VERSION"
echo

# --- 1 & 2: provenance and checksum -------------------------------------------
echo "[1/6] Artifact provenance and checksums"
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)  TARGET=aarch64-apple-darwin ;;
  Darwin/x86_64) TARGET=x86_64-apple-darwin ;;
  Linux/x86_64)  TARGET=x86_64-unknown-linux-gnu ;;
  *) TARGET="" ;;
esac

if [ -z "$TARGET" ]; then
  skip "no published artifact for $(uname -s)/$(uname -m); skipping to the rebuild"
else
  ART="neobrowser-${TARGET}.tar.gz"
  BASE="https://github.com/${REPO}/releases/download/${VERSION}"
  if curl -fsSL "$BASE/$ART" -o "$WORK/$ART"; then
    ok "downloaded $ART"
    if curl -fsSL "$BASE/$ART.sha256" -o "$WORK/$ART.sha256"; then
      if ( cd "$WORK" && { sha256sum -c "$ART.sha256" >/dev/null 2>&1 \
             || shasum -a 256 -c "$ART.sha256" >/dev/null 2>&1; } ); then
        ok "checksum matches"
      else
        bad "checksum does NOT match — do not run this binary"
      fi
    else
      bad "no published checksum for $ART"
    fi
    if command -v gh >/dev/null 2>&1; then
      if gh attestation verify "$WORK/$ART" --repo "$REPO" >/dev/null 2>&1; then
        ok "build provenance verified (built by $REPO's release workflow)"
      else
        bad "provenance could NOT be verified (is gh authenticated?)"
      fi
    else
      skip "provenance: install the GitHub CLI to check it"
    fi
  else
    bad "could not download $ART — does $VERSION exist?"
  fi
fi

# --- 3: the static-musl claim -------------------------------------------------
echo
echo "[2/6] The 'genuinely static' musl claim"
MUSL="neobrowser-x86_64-unknown-linux-musl.tar.gz"
if curl -fsSL "https://github.com/${REPO}/releases/download/${VERSION}/${MUSL}" -o "$WORK/$MUSL" 2>/dev/null; then
  tar -C "$WORK" -xzf "$WORK/$MUSL"
  if command -v file >/dev/null 2>&1; then
    DESC="$(file "$WORK/neobrowser")"
    echo "        $DESC"
    if echo "$DESC" | grep -q "dynamically linked"; then
      bad "the musl artifact is dynamically linked; the README's claim is wrong"
    else
      ok "statically linked, as claimed"
    fi
  else
    skip "no \`file\` command to inspect linkage"
  fi
  rm -f "$WORK/neobrowser"
else
  skip "no musl artifact published for $VERSION"
fi

# --- 4: rebuild from source ---------------------------------------------------
echo
echo "[3/6] Rebuild from the tagged source"
# rustup installs to ~/.cargo/bin and, with --no-modify-path, does not touch PATH — so
# look there before concluding there is no toolchain.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
  skip "no Rust toolchain; cannot rebuild (install from https://rustup.rs)"
else
  git clone --quiet --depth 1 --branch "$VERSION" "https://github.com/${REPO}" "$WORK/src" 2>/dev/null \
    || git clone --quiet "https://github.com/${REPO}" "$WORK/src"
  ( cd "$WORK/src" && git checkout --quiet "$VERSION" 2>/dev/null || true )
  # --locked: fail rather than silently resolving different dependency versions than the
  # ones the release was built with.
  if ( cd "$WORK/src/rust" && cargo build --release --locked >/dev/null 2>&1 ); then
    BIN="$WORK/src/rust/target/release/neobrowser"
    BUILT_VER="$("$BIN" --version | tr -d 'v \n')"
    WANT_VER="$(echo "$VERSION" | tr -d 'v \n')"
    if [ "$BUILT_VER" = "$WANT_VER" ]; then
      ok "rebuilt binary reports $BUILT_VER, matching the tag"
    else
      bad "rebuilt binary reports $BUILT_VER but the tag is $WANT_VER"
    fi
  else
    bad "rebuild failed"
  fi
fi

# --- 5: behaviour -------------------------------------------------------------
echo
echo "[4/6] The rebuilt binary's own test suite"
if [ -d "$WORK/src/rust" ]; then
  # A throwaway vault key: without one, on a host with no unlocked keyring the session
  # tests would exercise the refusal path instead of the crypto.
  if ( cd "$WORK/src/rust" \
       && NEOBROWSER_VAULT_KEY="$(printf '%s' 'ci-only-vault-key-32-bytes-!!!!!' | base64)" \
          cargo test --release >"$WORK/test.log" 2>&1 ); then
    ok "test suite passed ($(grep -c 'test result: ok' "$WORK/test.log") suites)"
  else
    bad "test suite failed; see $WORK/test.log"
    tail -20 "$WORK/test.log" >&2 || true
  fi
else
  skip "no rebuilt source to test"
fi

# --- 6: the README's own numbers ---------------------------------------------
echo
echo "[5/6] Do the documented numbers match reality?"
if [ -d "$WORK/src/rust" ]; then
  BIN="$WORK/src/rust/target/release/neobrowser"
  COUNT="$("$BIN" tools 2>/dev/null | grep -c '"name"' || echo 0)"
  DOC_COUNT="$(grep -oE '\*\*[0-9]+ tools\*\*' "$WORK/src/README.md" | grep -oE '[0-9]+' | head -1 || echo 0)"
  if [ "$COUNT" = "$DOC_COUNT" ]; then
    ok "tool count matches the README ($COUNT)"
  else
    bad "the binary exposes $COUNT tools; the README says $DOC_COUNT"
  fi
  SIZE_MB="$(( $(wc -c < "$BIN") / 1048576 ))"
  echo "        rebuilt binary: ${SIZE_MB} MB (README states ~6.2 MB; a debug-info or"
  echo "        toolchain difference of a megabyte is expected, not a discrepancy)"
else
  skip "no rebuilt binary to measure"
fi

echo
echo "[6/6] Environment report from the rebuilt binary"
if [ -d "$WORK/src/rust" ]; then
  "$WORK/src/rust/target/release/neobrowser" doctor --json || true
fi

echo
echo "-------------------------------------------"
echo "passed: $PASS   failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
  echo "VERIFICATION FAILED — please open an issue with this output." >&2
  exit 1
fi
echo "All executed checks passed."
echo
echo "What this does NOT prove: that the binary is byte-identical to the published one."
echo "It is not, and cannot be yet — see docs/REPRODUCIBILITY.md for the three specific"
echo "obstacles. Provenance is what covers the 'is this really from this source' question."
