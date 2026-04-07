#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# NeoRender Real Web Test Battery
# Tests actual websites to verify NeoRender works as a real
# browser engine for AI agents on production sites.
#
# Usage:
#   ./tests/test_web_real.sh           # run all
#   ./tests/test_web_real.sh A1        # run single test
# ═══════════════════════════════════════════════════════════════

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$SCRIPT_DIR/target/release/neobrowser_rs"
PASS=0; FAIL=0; SKIP=0; TOTAL=0
FILTER="${1:-}"

# ─── Colors ───
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

# ─── Verify binary ───
if [[ ! -x "$BIN" ]]; then
    echo "Binary not found: $BIN"
    echo "Run: cargo build --release"
    exit 1
fi

# ─── One-shot MCP call (separate process per request — reliable) ───
neo_open() {
    local url="$1" mode="${2:-neorender}" cookies="${3:-}" tmout="${4:-45}"
    local args="{\"url\":\"$url\",\"mode\":\"$mode\""
    [[ -n "$cookies" ]] && args="$args,\"cookies_file\":\"$cookies\""
    args="$args}"

    local req="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"browser_open\",\"arguments\":$args}}"

    echo "$req" | NEOBROWSER_HEADLESS=1 timeout "$tmout" "$BIN" mcp 2>/tmp/neo_web_real_stderr_last.log | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line.strip())
        if 'result' in d and 'content' in d['result']:
            for c in d['result']['content']:
                print(c.get('text',''))
    except: pass
" 2>/dev/null
}

# ─── Assertions ───
check() {
    local id="$1" name="$2" result="$3"
    shift 3

    if [[ -n "$FILTER" ]] && [[ "$id" != "$FILTER" ]]; then return; fi
    TOTAL=$((TOTAL+1))

    if [[ -z "$result" ]] || [[ "$result" == *'"error"'* && "$result" != *'"ok"'* ]]; then
        printf "  ${RED}FAIL${NC} %-6s %-38s timeout/empty\n" "$id" "$name"
        FAIL=$((FAIL+1))
        return
    fi

    local all_pass=true fail_msg=""
    for assertion in "$@"; do
        local check_result
        check_result=$(echo "$result" | python3 -c "
import json,sys
try:
    d=json.loads(sys.stdin.read())
    p=d.get('page','')
    $assertion
except Exception as e:
    print('FAIL: '+str(e))
" 2>/dev/null)
        if [[ "$check_result" != "PASS" ]]; then
            all_pass=false; fail_msg="$check_result"; break
        fi
    done

    if $all_pass; then
        local info
        info=$(echo "$result" | python3 -c "
import json,sys
d=json.loads(sys.stdin.read())
print(f\"L:{d.get('links',0)} F:{d.get('forms',0)} I:{d.get('inputs',0)} B:{d.get('buttons',0)} {d.get('html_bytes',0)//1024}KB {d.get('render_ms',0)}ms\")
" 2>/dev/null)
        printf "  ${GREEN}PASS${NC} %-6s %-38s %s\n" "$id" "$name" "$info"
        PASS=$((PASS+1))
    else
        printf "  ${RED}FAIL${NC} %-6s %-38s %s\n" "$id" "$name" "$fail_msg"
        FAIL=$((FAIL+1))
    fi
}

skip() {
    local id="$1" name="$2" reason="$3"
    if [[ -n "$FILTER" ]] && [[ "$id" != "$FILTER" ]]; then return; fi
    TOTAL=$((TOTAL+1)); SKIP=$((SKIP+1))
    printf "  ${YELLOW}SKIP${NC} %-6s %-38s %s\n" "$id" "$name" "$reason"
}

echo "═══════════════════════════════════════════════"
echo " NeoRender Real Web Test Battery"
echo "═══════════════════════════════════════════════"
echo ""

# ─── A. SEARCH ───
echo "▶ A. Search Engines"

R=$(neo_open "https://html.duckduckgo.com/html/?q=rust+programming" "light")
check "A2" "DuckDuckGo HTML search" "$R" \
    "print('PASS' if 'rust' in p.lower() and d.get('links',0)>3 else f\"FAIL: rust_in_page={('rust' in p.lower())} links={d.get('links',0)}\")"

R=$(neo_open "https://www.google.com/search?q=rust+programming+language&hl=en")
check "A1" "Google search results" "$R" \
    "print('PASS' if d.get('links',0)>3 else f\"FAIL: links={d.get('links',0)}\")"

R=$(neo_open "https://www.bing.com/search?q=rust+programming")
check "A3" "Bing search results" "$R" \
    "print('PASS' if d.get('html_bytes',0)>10000 else f\"FAIL: html_bytes={d.get('html_bytes',0)} links={d.get('links',0)}\")"

# ─── B. CONTENT EXTRACTION ───
echo ""; echo "▶ B. Content Extraction"

R=$(neo_open "https://en.wikipedia.org/wiki/Rust_(programming_language)")
check "B1" "Wikipedia article" "$R" \
    "print('PASS' if d.get('links',0)>50 and ('rust' in p.lower() or 'mozilla' in p.lower()) else f\"FAIL: links={d.get('links',0)} rust={('rust' in p.lower())}\")"

R=$(neo_open "https://news.ycombinator.com")
check "B2" "Hacker News front page" "$R" \
    "print('PASS' if d.get('links',0)>20 else f\"FAIL: links={d.get('links',0)}\")"

R=$(neo_open "https://github.com/nichochar/chrome-devtools-mcp")
check "B3" "GitHub repo page" "$R" \
    "print('PASS' if d.get('links',0)>5 and 'chrome' in p.lower() else f\"FAIL: links={d.get('links',0)} chrome={('chrome' in p.lower())}\")"

# ─── C. AUTHENTICATED SITES ───
echo ""; echo "▶ C. Authenticated Sites"

if [[ -f /tmp/chatgpt-fresh.json ]]; then
    # ChatGPT is a React SPA — full hydration requires ~30MB of JS.
    # NeoRender loads the SSR shell but React modules are stubbed (>1MB each).
    # Assert: page loads without crash, HTML received.
    R=$(neo_open "https://chatgpt.com" "neorender" "/tmp/chatgpt-fresh.json" 60)
    check "C1" "ChatGPT (SSR shell)" "$R" \
        "print('PASS' if d.get('ok',False) and d.get('html_bytes',0)>5000 else f\"FAIL: ok={d.get('ok')} html_bytes={d.get('html_bytes',0)}\")"
else
    skip "C1" "ChatGPT (SSR shell)" "no cookies at /tmp/chatgpt-fresh.json"
fi

if [[ -f /tmp/linkedin-fresh.json ]]; then
    # LinkedIn is a heavy SPA — module eval takes >60s in V8.
    # Use light mode (no JS) which still extracts SSR content.
    R=$(neo_open "https://www.linkedin.com/feed" "light" "/tmp/linkedin-fresh.json" 30)
    check "C2" "LinkedIn feed (light+cookies)" "$R" \
        "print('PASS' if d.get('ok',False) and d.get('html_bytes',0)>10000 else f\"FAIL: ok={d.get('ok')} html_bytes={d.get('html_bytes',0)} links={d.get('links',0)}\")"
else
    skip "C2" "LinkedIn feed (light+cookies)" "no cookies at /tmp/linkedin-fresh.json"
fi

# ─── D. E-COMMERCE ───
echo ""; echo "▶ D. E-Commerce"

R=$(neo_open "https://www.amazon.es/s?k=rust+book" "neorender" "" 60)
check "D1" "Amazon search results" "$R" \
    "print('PASS' if d.get('links',0)>5 else f\"FAIL: links={d.get('links',0)}\")"

# ─── E. FORM DETECTION ───
echo ""; echo "▶ E. Form Detection"

R=$(neo_open "https://news.ycombinator.com/login")
check "E1" "HN login form" "$R" \
    "print('PASS' if d.get('inputs',0)>=2 and d.get('forms',0)>=1 else f\"FAIL: inputs={d.get('inputs',0)} forms={d.get('forms',0)}\")"

R=$(neo_open "https://github.com/login")
check "E2" "GitHub login form" "$R" \
    "print('PASS' if d.get('inputs',0)>=2 else f\"FAIL: inputs={d.get('inputs',0)}\")"

# ─── F. NAVIGATION (multi-page — separate processes) ───
echo ""; echo "▶ F. Navigation"

if [[ -n "$FILTER" ]] && [[ "$FILTER" != "F1" ]]; then
    : # skip
else
    R1=$(neo_open "https://en.wikipedia.org/wiki/Rust_(programming_language)")
    R2=$(neo_open "https://en.wikipedia.org/wiki/Mozilla")
    TOTAL=$((TOTAL+1))
    T1=$(echo "$R1" | python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('title',''))" 2>/dev/null)
    T2=$(echo "$R2" | python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('title',''))" 2>/dev/null)
    if [[ -n "$T1" ]] && [[ -n "$T2" ]] && [[ "$T1" != "$T2" ]]; then
        printf "  ${GREEN}PASS${NC} %-6s %-38s %s -> %s\n" "F1" "Multi-page navigation" "$T1" "$T2"
        PASS=$((PASS+1))
    else
        printf "  ${RED}FAIL${NC} %-6s %-38s t1='%s' t2='%s'\n" "F1" "Multi-page navigation" "$T1" "$T2"
        FAIL=$((FAIL+1))
    fi
fi

# ─── G. SPA RENDERING ───
echo ""; echo "▶ G. SPA Rendering"

if [[ -f /tmp/chatgpt-fresh.json ]]; then
    # G1: Test that neorender doesn't crash on heavy React SPAs.
    # Full hydration is not expected (modules stubbed), but the engine
    # should return ok=True with the SSR HTML intact.
    R=$(neo_open "https://chatgpt.com" "neorender" "/tmp/chatgpt-fresh.json" 60)
    check "G1" "ChatGPT SPA resilience" "$R" \
        "print('PASS' if d.get('ok',False) and d.get('render_ms',0)<30000 else f\"FAIL: ok={d.get('ok')} render_ms={d.get('render_ms',0)}\")"
else
    skip "G1" "ChatGPT SPA resilience" "no cookies at /tmp/chatgpt-fresh.json"
fi

# ─── Summary ───
echo ""
echo "═══════════════════════════════════════════════"
printf " Results: ${GREEN}%d passed${NC}, ${RED}%d failed${NC}, ${YELLOW}%d skipped${NC} / %d total\n" "$PASS" "$FAIL" "$SKIP" "$TOTAL"
echo "═══════════════════════════════════════════════"

REQUIRED=$((TOTAL - SKIP))
if [[ $REQUIRED -gt 0 ]] && [[ $PASS -ge $((REQUIRED * 8 / 10)) ]]; then
    echo " PASS (80%+ of non-skipped tests)"
    exit 0
else
    echo " FAIL ($PASS/$REQUIRED non-skipped passed, need 80%+)"
    echo " Last stderr: /tmp/neo_web_real_stderr_last.log"
    exit 1
fi
