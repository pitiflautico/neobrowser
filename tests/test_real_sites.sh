#!/bin/bash
# NeoRender V2 — Real Site Validation (Fase F / R9.3)
# Tests top 10 sites for content extraction quality
#
# Gate: 8/10 must pass
# Per-site criteria:
#   - Exit code 0
#   - Content > 100 chars
#   - At least 1 link (href) found
#   - Completes within 30s

set -o pipefail

BINARY="$(cd "$(dirname "$0")/.." && pwd)/target/release/neorender"
OUTDIR="/tmp/neorender-v2-site-tests"
TIMEOUT=30
MIN_CHARS=100
MIN_LINKS=1
GATE=8

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

# Sites to test
declare -a SITES=(
  "https://en.wikipedia.org/wiki/Rust_(programming_language)"
  "https://github.com/nicbarker/clay"
  "https://news.ycombinator.com"
  "https://developer.mozilla.org/en-US/docs/Web/JavaScript"
  "https://stackoverflow.com/questions/tagged/rust"
  "https://old.reddit.com"
  "https://www.amazon.com"
  "https://www.nytimes.com"
  "https://www.youtube.com"
  "https://x.com"
)

declare -a LABELS=(
  "Wikipedia"
  "GitHub"
  "HackerNews"
  "MDN"
  "StackOverflow"
  "Reddit"
  "Amazon"
  "NYTimes"
  "YouTube"
  "Twitter-X"
)

# ── Setup ──────────────────────────────────────────────────────────────
rm -rf "$OUTDIR"
mkdir -p "$OUTDIR"

if [[ ! -x "$BINARY" ]]; then
  echo -e "${RED}ERROR: Binary not found at $BINARY${NC}"
  echo "Run: cargo build --release"
  exit 1
fi

# ── Run tests ──────────────────────────────────────────────────────────
PASSED=0
FAILED=0
TOTAL=${#SITES[@]}

printf "\n${BOLD}NeoRender V2 — Real Site Validation${NC}\n"
printf "Binary:  %s\n" "$BINARY"
printf "Output:  %s\n" "$OUTDIR"
printf "Gate:    %d/%d\n" "$GATE" "$TOTAL"
printf "Timeout: %ds per site\n\n" "$TIMEOUT"
printf "%-15s %-6s %8s %8s %6s %s\n" "SITE" "STATUS" "TIME(s)" "CHARS" "LINKS" "NOTES"
printf "%-15s %-6s %8s %8s %6s %s\n" "----" "------" "-------" "-----" "-----" "-----"

for i in "${!SITES[@]}"; do
  url="${SITES[$i]}"
  label="${LABELS[$i]}"
  out_file="$OUTDIR/${label}.json"
  err_file="$OUTDIR/${label}.stderr"
  notes=""

  # Time and run
  start_ts=$(python3 -c 'import time; print(time.time())')
  timeout "$TIMEOUT" "$BINARY" see "$url" > "$out_file" 2> "$err_file"
  exit_code=$?
  end_ts=$(python3 -c 'import time; print(time.time())')
  elapsed=$(python3 -c "print(f'{${end_ts} - ${start_ts}:.1f}')")

  # Measure content
  if [[ -f "$out_file" ]]; then
    chars=$(wc -c < "$out_file" | tr -d ' ')
    links=$(grep -o '"href"' "$out_file" 2>/dev/null | wc -l | tr -d ' ')
  else
    chars=0
    links=0
  fi

  # Evaluate
  ok=true

  if [[ $exit_code -eq 124 ]]; then
    notes="TIMEOUT"
    ok=false
  elif [[ $exit_code -ne 0 ]]; then
    notes="exit=$exit_code"
    ok=false
  fi

  if [[ $chars -lt $MIN_CHARS ]]; then
    notes="${notes:+$notes, }chars<$MIN_CHARS"
    ok=false
  fi

  if [[ $links -lt $MIN_LINKS ]]; then
    notes="${notes:+$notes, }no links"
    ok=false
  fi

  if $ok; then
    status="${GREEN}PASS${NC}"
    PASSED=$((PASSED + 1))
  else
    status="${RED}FAIL${NC}"
    FAILED=$((FAILED + 1))
  fi

  printf "%-15s $(echo -e "$status")   %7s %8s %6s %s\n" "$label" "${elapsed}s" "$chars" "$links" "$notes"
done

# ── Summary ────────────────────────────────────────────────────────────
printf "\n${BOLD}Results: %d/%d passed${NC}\n" "$PASSED" "$TOTAL"

if [[ $PASSED -ge $GATE ]]; then
  printf "${GREEN}GATE PASSED${NC} (needed %d/%d)\n\n" "$GATE" "$TOTAL"
  exit 0
else
  printf "${RED}GATE FAILED${NC} (needed %d/%d, got %d)\n\n" "$GATE" "$TOTAL" "$PASSED"
  # Show stderr for failed sites
  echo "--- Failure details ---"
  for i in "${!LABELS[@]}"; do
    label="${LABELS[$i]}"
    err_file="$OUTDIR/${label}.stderr"
    out_file="$OUTDIR/${label}.json"
    chars=$(wc -c < "$out_file" 2>/dev/null | tr -d ' ')
    if [[ ${chars:-0} -lt $MIN_CHARS ]]; then
      echo ""
      echo "[$label] stderr:"
      tail -5 "$err_file" 2>/dev/null
    fi
  done
  exit 1
fi
