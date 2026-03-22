#!/usr/bin/env bash
# Compare Chrome CDP vs NeoRender on the same URL.
#
# Usage:
#   ./scripts/compare_chrome_neo.sh <url>
#   ./scripts/compare_chrome_neo.sh https://chatgpt.com
#   ./scripts/compare_chrome_neo.sh https://example.com --timeout 15
#
# Prerequisites:
#   - NeoRender binary built (cargo build --release)
#   - For Chrome comparison: Chrome running with --remote-debugging-port=9222
#   - pip install websocket-client (optional, for full Chrome diagnostics)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

export NEORENDER="${NEORENDER:-$ROOT_DIR/target/release/neorender}"

# Check neorender binary
if [ ! -f "$NEORENDER" ]; then
    echo "NeoRender binary not found at $NEORENDER"
    echo "Building..."
    cargo build --release --manifest-path="$ROOT_DIR/Cargo.toml"
fi

# Check Chrome CDP availability
if curl -s http://localhost:9222/json/version >/dev/null 2>&1; then
    echo "[chrome] CDP available on :9222"
else
    echo "[chrome] Not running. Starting headless Chrome..."
    /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
        --remote-debugging-port=9222 \
        --headless=new \
        --disable-gpu \
        --no-first-run \
        --user-data-dir=/tmp/chrome-debug-profile \
        &>/dev/null &
    CHROME_PID=$!
    echo "[chrome] Started (pid=$CHROME_PID), waiting 3s..."
    sleep 3

    if ! curl -s http://localhost:9222/json/version >/dev/null 2>&1; then
        echo "[chrome] Failed to start. Continuing without Chrome."
    fi
fi

# Run comparison
python3 "$SCRIPT_DIR/compare.py" "$@"
