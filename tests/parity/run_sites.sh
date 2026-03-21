#!/bin/bash
# R2.7 Parity Harness — Run parity check against real sites.
# Usage: ./run_sites.sh [tolerance_percent]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOLERANCE="${1:-10}"

SITES=(
  "https://news.ycombinator.com"
  "https://en.wikipedia.org/wiki/Main_Page"
  "https://old.reddit.com"
)

echo "=== Site Parity (tolerance ${TOLERANCE}%) ==="
echo ""

total=0
ok=0
warn=0
fail=0

for url in "${SITES[@]}"; do
  total=$((total + 1))
  domain=$(echo "$url" | sed 's|https\?://||; s|/.*||')
  echo -n "[$domain] "

  result=$("$SCRIPT_DIR/run_parity.sh" "$url" "$TOLERANCE" 2>/dev/null)
  echo "$result"

  case "$result" in
    *"PARITY OK"*) ok=$((ok + 1)) ;;
    *COSMETIC*) warn=$((warn + 1)) ;;
    *CRITICAL*) fail=$((fail + 1)) ;;
    *) warn=$((warn + 1)) ;;  # unexpected output = warn
  esac

  echo ""
done

echo "=== Summary: $total sites — $ok ok, $warn warn, $fail critical ==="
