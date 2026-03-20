#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# NeoRender Real-Site Regression Tests
# Verifies NeoRender still works against 5 real websites.
# ═══════════════════════════════════════════════════════════════

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$SCRIPT_DIR/target/release/neobrowser_rs"
PASS=0
FAIL=0
TOTAL=0

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

if [[ ! -x "$BINARY" ]]; then
    echo "Binary not found: $BINARY"
    exit 1
fi

# ─── FIFO MCP session ───
MCP_PID=""
FIFO_IN=""
FIFO_OUT=""
REQ_ID=0

start_mcp() {
    FIFO_IN=$(mktemp -u /tmp/neo_reg_in.XXXXXX)
    FIFO_OUT=$(mktemp -u /tmp/neo_reg_out.XXXXXX)
    mkfifo "$FIFO_IN"
    mkfifo "$FIFO_OUT"
    NEOBROWSER_HEADLESS=1 "$BINARY" mcp < "$FIFO_IN" > "$FIFO_OUT" 2>/tmp/neo_regression_stderr.log &
    MCP_PID=$!
    exec 3>"$FIFO_IN"
    exec 4<"$FIFO_OUT"
    REQ_ID=0
    echo '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' >&3
    read -r -t 10 _resp <&4 || true
    echo '{"jsonrpc":"2.0","id":null,"method":"notifications/initialized","params":{}}' >&3
    sleep 0.2
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

call_tool() {
    local tool_name="$1"
    local tool_args="$2"
    REQ_ID=$((REQ_ID + 1))
    echo "{\"jsonrpc\":\"2.0\",\"id\":$REQ_ID,\"method\":\"tools/call\",\"params\":{\"name\":\"$tool_name\",\"arguments\":$tool_args}}" >&3
    local resp
    if read -r -t 60 resp <&4; then
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

run_site() {
    local name="$1" url="$2" assertion="$3"
    TOTAL=$((TOTAL+1))
    local result
    result=$(call_tool "browser_open" "{\"url\":\"$url\",\"mode\":\"neorender\"}")
    if [[ -z "$result" ]] || [[ "$result" == *'"error":"timeout"'* ]]; then
        printf "  ${RED}FAIL${NC} %-25s timeout\n" "$name"
        FAIL=$((FAIL+1))
        return
    fi
    local check
    check=$(echo "$result" | python3 -c "$assertion" 2>/dev/null)
    if [[ "$check" == "PASS" ]]; then
        printf "  ${GREEN}PASS${NC} %s\n" "$name"
        PASS=$((PASS+1))
    else
        printf "  ${RED}FAIL${NC} %-25s %s\n" "$name" "$check"
        FAIL=$((FAIL+1))
    fi
}

trap stop_mcp EXIT

echo ""
echo "═══════════════════════════════════════"
echo "  NeoRender Real-Site Regression"
echo "═══════════════════════════════════════"
echo ""

start_mcp

A_WIKI="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
t=d.get('title','')
print('PASS' if ok and 'Wikipedia' in t else 'FAIL: ok='+str(ok)+' title='+t[:60])
"

A_HN="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
p=d.get('page','')
print('PASS' if ok and ('Hacker News' in p or 'hacker' in p.lower()) else 'FAIL: ok='+str(ok)+' page[:100]='+p[:100])
"

A_REDDIT="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
links=d.get('links',0)
print('PASS' if ok and links>5 else 'FAIL: ok='+str(ok)+' links='+str(links))
"

A_GITHUB="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
p=d.get('page','')
print('PASS' if ok and ('GitHub' in p or 'github' in p.lower()) else 'FAIL: ok='+str(ok)+' page[:100]='+p[:100])
"

A_AMAZON="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
p=d.get('page','')
t=d.get('title','')
print('PASS' if ok and ('Amazon' in t or 'amazon' in p.lower()) else 'FAIL: ok='+str(ok)+' title='+t[:60])
"

A_NPM="
import json,sys
d=json.loads(sys.stdin.read())
ok=d.get('ok',False)
p=d.get('page','')
print('PASS' if ok and ('npm' in p.lower() or 'package' in p.lower()) else 'FAIL: ok='+str(ok)+' page[:100]='+p[:100])
"

run_site "Wikipedia"   "https://en.wikipedia.org/wiki/Main_Page"  "$A_WIKI"
run_site "Hacker News" "https://news.ycombinator.com/"            "$A_HN"
run_site "Reddit"      "https://old.reddit.com/"                  "$A_REDDIT"
run_site "GitHub"      "https://github.com/trending"              "$A_GITHUB"
run_site "npmjs"       "https://www.npmjs.com/"                   "$A_NPM"

echo ""
echo "═══════════════════════════════════════"
printf "  Results: ${GREEN}%d${NC}/%d passed, ${RED}%d${NC} failed\n" "$PASS" "$TOTAL" "$FAIL"
echo "═══════════════════════════════════════"
echo ""

if [[ "$FAIL" -gt 0 ]]; then
    echo "Stderr log: /tmp/neo_regression_stderr.log"
    exit 1
fi
