#!/usr/bin/env bash
# NeoRender V2 — 10-site test battery + 3 interaction tests
# Usage: ./tests/test_sites.sh [--timeout N]
set -u

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
NEORENDER="$SCRIPT_DIR/target/release/neorender"
TIMEOUT="${1:-45}"
RESULTS_FILE="/tmp/neorender-test-battery.md"

if [[ ! -x "$NEORENDER" ]]; then
  echo "ERROR: Binary not found at $NEORENDER" >&2
  exit 1
fi

# Colors
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

pass() { printf "${GREEN}PASS${NC} %s\n" "$1"; }
fail() { printf "${RED}FAIL${NC} %s\n" "$1"; }
warn() { printf "${YELLOW}WARN${NC} %s\n" "$1"; }

declare -a SITE_RESULTS=()
declare -a INTERACT_RESULTS=()

# ============================================================
# SEE tests — 10 sites
# ============================================================

test_see() {
  local url="$1"
  local label="$2"
  local min_nodes="$3"
  local expect_tags="$4"

  local tmpfile="/tmp/neorender-see-${label}.json"

  printf "[%-14s] %s ... " "$label" "$url"

  local ok=0
  timeout "$TIMEOUT" "$NEORENDER" see "$url" 2>/dev/null > "$tmpfile" && ok=1 || ok=$?

  # If file is empty or missing, it's a failure
  if [[ ! -s "$tmpfile" ]]; then
    fail "$label — timeout or crash (exit=$ok)"
    SITE_RESULTS+=("| $label | \`$url\` | FAIL | 0 | - | timeout/crash | - |")
    return
  fi

  local result
  result=$(python3 << PYEOF
import json, sys
try:
    d = json.load(open("$tmpfile"))
except Exception as e:
    print(f"PARSE_ERROR|0|0|json error: {e}|none|parse error")
    sys.exit(0)

nodes = d.get("wom", {}).get("nodes", [])
node_count = len(nodes)
title = d.get("title", "")
render_ms = d.get("render_ms", 0)
errors = d.get("errors", [])

from collections import Counter
tags = Counter(n.get("tag","") for n in nodes)
tag_list = ",".join(f"{k}:{v}" for k,v in tags.most_common(8))

expected = "$expect_tags".split(",")
missing_tags = [t.strip() for t in expected if t.strip() and tags.get(t.strip(), 0) == 0]

status = "PASS"
issues = []
if node_count < $min_nodes:
    status = "FAIL"
    issues.append(f"nodes {node_count} < $min_nodes")
if missing_tags:
    if status == "PASS":
        status = "WARN"
    issues.append(f"missing: {','.join(missing_tags)}")
if errors:
    issues.append(f"{len(errors)} js-errors")

issue_str = "; ".join(issues) if issues else "ok"
print(f"{status}|{node_count}|{render_ms}|{title}|{tag_list}|{issue_str}")
PYEOF
)

  local status=$(echo "$result" | cut -d'|' -f1)
  local node_count=$(echo "$result" | cut -d'|' -f2)
  local render_ms=$(echo "$result" | cut -d'|' -f3)
  local title=$(echo "$result" | cut -d'|' -f4)
  local tag_list=$(echo "$result" | cut -d'|' -f5)
  local issues=$(echo "$result" | cut -d'|' -f6)

  local render_s=$(python3 -c "print(f'{${render_ms}/1000:.1f}s')")

  case "$status" in
    PASS) pass "$label — ${node_count} nodes, ${render_s}, title='${title}'" ;;
    WARN) warn "$label — ${node_count} nodes, ${render_s} — $issues" ;;
    FAIL) fail "$label — ${node_count} nodes, ${render_s} — $issues" ;;
  esac

  SITE_RESULTS+=("| $label | \`$url\` | $status | $node_count | $render_s | $issues | $tag_list |")
}

echo "=========================================="
echo "NeoRender V2 — 10-Site Test Battery"
echo "=========================================="
echo ""
echo "--- SEE Tests (content extraction) ---"
echo ""

test_see "https://news.ycombinator.com"  "HN"             200  "a,input,table"
test_see "https://www.mercadona.es"       "Mercadona"      100  "input,button"
test_see "https://react.dev"              "React.dev"      200  "a,input"
test_see "https://github.com"             "GitHub"         400  "a,input"
test_see "https://en.wikipedia.org"       "Wikipedia"      1000 "a"
test_see "https://vuejs.org"              "Vue.js"         200  "a,img"
test_see "https://stackoverflow.com"      "StackOverflow"  400  "a,input"
test_see "https://svelte.dev"             "Svelte"         100  "a"
test_see "https://vercel.com"             "Vercel"         100  "a"
test_see "https://duckduckgo.com/?q=test" "DDG-Search"     50   "a"

# ============================================================
# INTERACT tests — 3 sites
# ============================================================

echo ""
echo "--- INTERACT Tests ---"
echo ""

test_interact() {
  local label="$1"
  local url="$2"
  local commands="$3"
  local expect_pattern="$4"

  printf "[%-14s interact] ... " "$label"

  local tmpfile="/tmp/neorender-interact-${label}.txt"
  echo "$commands" | timeout "$TIMEOUT" "$NEORENDER" interact "$url" > "$tmpfile" 2>/dev/null || true

  if [[ ! -s "$tmpfile" ]]; then
    fail "$label interact — no output"
    INTERACT_RESULTS+=("| $label | FAIL | No output |")
    return
  fi

  if grep -qiE "$expect_pattern" "$tmpfile" 2>/dev/null; then
    local match=$(grep -iE "$expect_pattern" "$tmpfile" | head -1 | cut -c1-100)
    pass "$label interact — matched: $match"
    INTERACT_RESULTS+=("| $label | PASS | Matched: \`$(echo "$match" | sed 's/|/\\|/g')\` |")
  else
    local preview=$(head -3 "$tmpfile" | tr '\n' ' ' | cut -c1-120)
    fail "$label interact — pattern '$expect_pattern' not found"
    INTERACT_RESULTS+=("| $label | FAIL | Pattern \`$expect_pattern\` not in output. Got: \`${preview}\` |")
  fi
}

test_interact "HN" "https://news.ycombinator.com" \
  "$(printf 'find Hacker News\neval document.title\nquit\n')" \
  "Hacker News"

test_interact "Mercadona" "https://www.mercadona.es" \
  "$(printf 'find postal\neval document.querySelectorAll(\"button\").length\nquit\n')" \
  "[0-9]"

test_interact "Wikipedia" "https://en.wikipedia.org" \
  "$(printf 'find English\neval document.querySelectorAll(\"a\").length\nquit\n')" \
  "[0-9]"

# ============================================================
# Generate report
# ============================================================

echo ""
echo "=========================================="
echo "Writing results to $RESULTS_FILE"
echo "=========================================="

{
echo "# NeoRender V2 — 10-Site Test Battery Results"
echo ""
echo "**Date**: $(date '+%Y-%m-%d %H:%M:%S')"
echo "**Timeout**: ${TIMEOUT}s per site"
echo ""
echo "## SEE Tests (Content Extraction)"
echo ""
echo "| Site | URL | Status | Nodes | Time | Notes | Tags |"
echo "|------|-----|--------|-------|------|-------|------|"
for row in "${SITE_RESULTS[@]}"; do
  echo "$row"
done
echo ""
echo "## INTERACT Tests"
echo ""
echo "| Site | Status | Details |"
echo "|------|--------|---------|"
for row in "${INTERACT_RESULTS[@]}"; do
  echo "$row"
done
echo ""

# Count results
total_see=${#SITE_RESULTS[@]}
pass_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| PASS |" || true)
warn_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| WARN |" || true)
fail_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| FAIL |" || true)
total_int=${#INTERACT_RESULTS[@]}
pass_int=$(printf '%s\n' "${INTERACT_RESULTS[@]}" | grep -c "| PASS |" || true)
fail_int=$(printf '%s\n' "${INTERACT_RESULTS[@]}" | grep -c "| FAIL |" || true)

echo "## Summary"
echo ""
echo "- **SEE**: ${pass_see} pass, ${warn_see} warn, ${fail_see} fail / ${total_see} total"
echo "- **INTERACT**: ${pass_int} pass, ${fail_int} fail / ${total_int} total"
} > "$RESULTS_FILE"

# Print summary
total_see=${#SITE_RESULTS[@]}
pass_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| PASS |" || true)
warn_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| WARN |" || true)
fail_see=$(printf '%s\n' "${SITE_RESULTS[@]}" | grep -c "| FAIL |" || true)
total_int=${#INTERACT_RESULTS[@]}
pass_int=$(printf '%s\n' "${INTERACT_RESULTS[@]}" | grep -c "| PASS |" || true)
fail_int=$(printf '%s\n' "${INTERACT_RESULTS[@]}" | grep -c "| FAIL |" || true)

echo ""
echo "Summary: SEE ${pass_see}/${total_see} pass (${warn_see} warn, ${fail_see} fail) | INTERACT ${pass_int}/${total_int} pass"
echo "Report: $RESULTS_FILE"
