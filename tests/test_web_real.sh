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
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BOLD='\033[1m'; NC='\033[0m'

# ─── Verify binary ───
if [[ ! -x "$BIN" ]]; then
    echo "Binary not found: $BIN"
    echo "Run: cargo build --release"
    exit 1
fi

# ─── FIFO MCP session ───
MCP_PID=""
FIFO_IN=""
FIFO_OUT=""
REQ_ID=0

start_mcp() {
    FIFO_IN=$(mktemp -u /tmp/neo_web_in.XXXXXX)
    FIFO_OUT=$(mktemp -u /tmp/neo_web_out.XXXXXX)
    mkfifo "$FIFO_IN"
    mkfifo "$FIFO_OUT"

    NEOBROWSER_HEADLESS=1 "$BIN" mcp < "$FIFO_IN" > "$FIFO_OUT" 2>/tmp/neo_web_real_stderr.log &
    MCP_PID=$!
    exec 3>"$FIFO_IN"
    exec 4<"$FIFO_OUT"
    REQ_ID=0

    # Initialize handshake
    echo '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' >&3
    read -r -t 10 _resp <&4 || true
    echo '{"jsonrpc":"2.0","id":null,"method":"notifications/initialized","params":{}}' >&3
    sleep 0.3
}

stop_mcp() {
    if [[ -n "$MCP_PID" ]] && kill -0 "$MCP_PID" 2>/dev/null; then
        exec 3>&- 2>/dev/null || true
        exec 4<&- 2>/dev/null || true
        kill "$MCP_PID" 2>/dev/null || true
        wait "$MCP_PID" 2>/dev/null || true
    fi
    [[ -n "$FIFO_IN" ]] && rm -f "$FIFO_IN"
    [[ -n "$FIFO_OUT" ]] && rm -f "$FIFO_OUT"
    MCP_PID=""
}

trap stop_mcp EXIT

call_tool() {
    local tool_name="$1"
    local tool_args="$2"
    local tmout="${3:-30}"
    REQ_ID=$((REQ_ID + 1))

    local req="{\"jsonrpc\":\"2.0\",\"id\":$REQ_ID,\"method\":\"tools/call\",\"params\":{\"name\":\"$tool_name\",\"arguments\":$tool_args}}"
    echo "$req" >&3

    local resp
    if read -r -t "$tmout" resp <&4; then
        echo "$resp" | python3 -c "
import json,sys
d=json.loads(sys.stdin.read())
if 'result' in d and 'content' in d['result']:
    for c in d['result']['content']:
        print(c.get('text',''))
elif 'error' in d:
    print(json.dumps(d['error']))
" 2>/dev/null
    else
        echo '{"error":"timeout"}'
    fi
}

neo_open() {
    local url="$1" mode="${2:-neorender}" cookies="${3:-}"
    local args="{\"url\":\"$url\",\"mode\":\"$mode\""
    [[ -n "$cookies" ]] && args="$args,\"cookies_file\":\"$cookies\""
    args="$args}"
    call_tool "browser_open" "$args" 30
}

# ─── Assertions ───
check() {
    local id="$1" name="$2" result="$3"
    shift 3
    # remaining args are python assertion snippets

    if [[ -n "$FILTER" ]] && [[ "$id" != "$FILTER" ]]; then
        return
    fi

    TOTAL=$((TOTAL+1))

    if [[ -z "$result" ]] || [[ "$result" == *'"error":"timeout"'* ]]; then
        printf "  ${RED}FAIL${NC} %-6s %-38s timeout/empty\n" "$id" "$name"
        FAIL=$((FAIL+1))
        return
    fi

    local all_pass=true
    local fail_msg=""
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
            all_pass=false
            fail_msg="$check_result"
            break
        fi
    done

    if $all_pass; then
        # Extract summary info
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

start_mcp

# ─── A. SEARCH ───
echo "▶ A. Search Engines"

R=$(neo_open "https://html.duckduckgo.com/html/?q=rust+programming" "light")
check "A2" "DuckDuckGo HTML search" "$R" \
    "print('PASS' if 'rust' in p.lower() and d.get('links',0)>3 else f\"FAIL: 'rust' in page={('rust' in p.lower())} links={d.get('links',0)}\")"

R=$(neo_open "https://www.google.com/search?q=rust+programming+language&hl=en")
check "A1" "Google search results" "$R" \
    "print('PASS' if d.get('links',0)>3 else f\"FAIL: links={d.get('links',0)}\")"

R=$(neo_open "https://www.bing.com/search?q=rust+programming")
check "A3" "Bing search results" "$R" \
    "print('PASS' if d.get('links',0)>5 else f\"FAIL: links={d.get('links',0)}\")"

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
    R=$(neo_open "https://chatgpt.com" "neorender" "/tmp/chatgpt-fresh.json")
    check "C1" "ChatGPT (authenticated)" "$R" \
        "print('PASS' if d.get('render_ms',99999)<15000 and (d.get('buttons',0)>0 or d.get('links',0)>0) else f\"FAIL: ms={d.get('render_ms',0)} btns={d.get('buttons',0)} links={d.get('links',0)}\")"
else
    skip "C1" "ChatGPT (authenticated)" "no cookies at /tmp/chatgpt-fresh.json"
fi

if [[ -f /tmp/linkedin-fresh.json ]]; then
    R=$(neo_open "https://www.linkedin.com/feed" "neorender" "/tmp/linkedin-fresh.json")
    check "C2" "LinkedIn feed (authenticated)" "$R" \
        "print('PASS' if d.get('links',0)>5 and len(p)>1000 else f\"FAIL: links={d.get('links',0)} page_len={len(p)}\")"
else
    skip "C2" "LinkedIn feed (authenticated)" "no cookies at /tmp/linkedin-fresh.json"
fi

# ─── D. E-COMMERCE ───
echo ""; echo "▶ D. E-Commerce"

R=$(neo_open "https://www.amazon.es/s?k=rust+book")
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

# ─── F. NAVIGATION (multi-page) ───
echo ""; echo "▶ F. Navigation"

R1=$(neo_open "https://en.wikipedia.org/wiki/Rust_(programming_language)")
R2=$(neo_open "https://en.wikipedia.org/wiki/Mozilla")
if [[ -n "$FILTER" ]] && [[ "$FILTER" != "F1" ]]; then
    : # skip
else
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
    R=$(neo_open "https://chatgpt.com" "neorender" "/tmp/chatgpt-fresh.json")
    check "G1" "ChatGPT React hydration" "$R" \
        "print('PASS' if d.get('ok',False) and (d.get('buttons',0)>0 or d.get('links',0)>2 or len(p)>500) else f\"FAIL: ok={d.get('ok')} btns={d.get('buttons',0)} links={d.get('links',0)} page_len={len(p)}\")"
else
    skip "G1" "ChatGPT React hydration" "no cookies at /tmp/chatgpt-fresh.json"
fi

stop_mcp

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
    echo " Stderr: /tmp/neo_web_real_stderr.log"
    exit 1
fi
