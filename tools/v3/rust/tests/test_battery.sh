#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# NeoRender Local HTML Test Battery
# Tests NeoRender V8 engine against local HTML pages.
#
# Usage:
#   ./tests/test_battery.sh           # run all tests
#   ./tests/test_battery.sh 3         # run single test
# ═══════════════════════════════════════════════════════════════

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$SCRIPT_DIR/target/release/neobrowser_rs"
DIR="$SCRIPT_DIR/tests/battery"
PASS=0
FAIL=0
TOTAL=0
HTTP_PID=""

# ─── Color helpers ───
RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

# ─── Parse test filter ───
FILTER=""
if [[ $# -ge 1 ]]; then
    FILTER="$1"
fi

# ─── Verify binary ───
if [[ ! -x "$BINARY" ]]; then
    echo "Binary not found: $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

# ─── Start local HTTP server ───
start_http() {
    python3 -m http.server 9877 -d "$DIR" >/dev/null 2>&1 &
    HTTP_PID=$!
    # Wait for server to be ready
    for i in $(seq 1 20); do
        if curl -s -o /dev/null http://localhost:9877/basic.html 2>/dev/null; then
            break
        fi
        sleep 0.2
    done
    if ! kill -0 "$HTTP_PID" 2>/dev/null; then
        echo "Failed to start HTTP server"
        exit 1
    fi
}

stop_http() {
    if [[ -n "$HTTP_PID" ]]; then
        kill "$HTTP_PID" 2>/dev/null || true
        wait "$HTTP_PID" 2>/dev/null || true
    fi
}

# ─── FIFO MCP session management (from browser_tests.sh pattern) ───
MCP_PID=""
FIFO_IN=""
FIFO_OUT=""
REQ_ID=0

start_mcp() {
    FIFO_IN=$(mktemp -u /tmp/neo_bat_in.XXXXXX)
    FIFO_OUT=$(mktemp -u /tmp/neo_bat_out.XXXXXX)
    mkfifo "$FIFO_IN"
    mkfifo "$FIFO_OUT"

    NEOBROWSER_HEADLESS=1 "$BINARY" mcp < "$FIFO_IN" > "$FIFO_OUT" 2>/tmp/neo_battery_stderr.log &
    MCP_PID=$!
    exec 3>"$FIFO_IN"
    exec 4<"$FIFO_OUT"
    REQ_ID=0

    # Initialize handshake
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

    local req="{\"jsonrpc\":\"2.0\",\"id\":$REQ_ID,\"method\":\"tools/call\",\"params\":{\"name\":\"$tool_name\",\"arguments\":$tool_args}}"
    echo "$req" >&3

    local resp
    if read -r -t 30 resp <&4; then
        # Extract inner tool result text
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

# ─── Test runner ───

run_test() {
    local num="$1" name="$2" url="$3"
    shift 3
    # remaining args are assertion commands

    if [[ -n "$FILTER" ]] && [[ "$num" != "$FILTER" ]]; then
        return
    fi

    TOTAL=$((TOTAL+1))

    local result
    result=$(call_tool "browser_open" "{\"url\":\"$url\",\"mode\":\"neorender\"}")

    if [[ -z "$result" ]] || [[ "$result" == *'"error"'* && "$result" != *'"ok"'* ]]; then
        printf "  ${RED}FAIL${NC} T%s: %-30s %s\n" "$num" "$name" "timeout/error"
        echo "    Raw: ${result:0:200}"
        FAIL=$((FAIL+1))
        return
    fi

    # Run assertions
    local all_pass=true
    local fail_msg=""
    for assertion in "$@"; do
        local check
        check=$(echo "$result" | python3 -c "$assertion" 2>/dev/null)
        if [[ "$check" != "PASS" ]]; then
            all_pass=false
            fail_msg="$check"
            break
        fi
    done

    if $all_pass; then
        printf "  ${GREEN}PASS${NC} T%s: %s\n" "$num" "$name"
        PASS=$((PASS+1))
    else
        printf "  ${RED}FAIL${NC} T%s: %-30s %s\n" "$num" "$name" "$fail_msg"
        FAIL=$((FAIL+1))
    fi
}

# ─── Cleanup on exit ───
cleanup() {
    stop_mcp
    stop_http
}
trap cleanup EXIT

# ═══════════════════════════════════════════════════════════════
echo ""
echo "═══════════════════════════════════════"
echo "  NeoRender Local Test Battery"
echo "═══════════════════════════════════════"
echo ""

start_http
start_mcp

# ─── Assertion snippets (Python one-liners reading JSON from stdin) ───

A_BASIC_TITLE="
import json,sys
d=json.loads(sys.stdin.read())
print('PASS' if d.get('title')=='Basic Test Page' else 'FAIL: title='+str(d.get('title','')))
"

A_BASIC_LINKS="
import json,sys
d=json.loads(sys.stdin.read())
n=d.get('links',0)
print('PASS' if n>=3 else 'FAIL: links='+str(n)+' (need>=3)')
"

A_FORMS_INPUTS="
import json,sys
d=json.loads(sys.stdin.read())
n=d.get('inputs',0)
print('PASS' if n>=4 else 'FAIL: inputs='+str(n)+' (need>=4)')
"

A_FORMS_BUTTONS="
import json,sys
d=json.loads(sys.stdin.read())
n=d.get('buttons',0)
print('PASS' if n>=2 else 'FAIL: buttons='+str(n)+' (need>=2)')
"

A_SPA_CONTENT="
import json,sys
d=json.loads(sys.stdin.read())
p=d.get('page','')
print('PASS' if 'SPA Loaded' in p or 'Dynamic content' in p else 'FAIL: no dynamic content in page')
"

A_TABLES_DATA="
import json,sys
d=json.loads(sys.stdin.read())
p=d.get('page','')
print('PASS' if 'Alice' in p and 'Bob' in p else 'FAIL: table data missing')
"

A_JS_SPEED="
import json,sys
d=json.loads(sys.stdin.read())
ms=d.get('render_ms',99999)
print('PASS' if ms<10000 else 'FAIL: render_ms='+str(ms))
"

A_JS_OK="
import json,sys
d=json.loads(sys.stdin.read())
print('PASS' if d.get('ok')==True else 'FAIL: ok='+str(d.get('ok','')))
"

A_REACT_CONTENT="
import json,sys
d=json.loads(sys.stdin.read())
p=d.get('page','')
print('PASS' if 'React-like App' in p else 'FAIL: no React content')
"

A_VUE_CONTENT="
import json,sys
d=json.loads(sys.stdin.read())
p=d.get('page','')
print('PASS' if 'Hello from Vue' in p and 'Item 1' in p else 'FAIL: no Vue content')
"

# ─── Run tests ───

run_test 1 "Basic HTML"         "http://localhost:9877/basic.html"      "$A_BASIC_TITLE" "$A_BASIC_LINKS"
run_test 2 "Forms"              "http://localhost:9877/forms.html"      "$A_FORMS_INPUTS" "$A_FORMS_BUTTONS"
run_test 3 "SPA (JS DOM)"       "http://localhost:9877/spa.html"        "$A_SPA_CONTENT"
run_test 4 "Tables + JSON-LD"   "http://localhost:9877/tables.html"     "$A_TABLES_DATA"
run_test 5 "JS Heavy (timers)"  "http://localhost:9877/js-heavy.html"   "$A_JS_OK" "$A_JS_SPEED"
run_test 6 "React-like"         "http://localhost:9877/react-like.html" "$A_REACT_CONTENT"
run_test 7 "Vue-like"           "http://localhost:9877/vue-like.html"   "$A_VUE_CONTENT"

# ─── Summary ───
echo ""
echo "═══════════════════════════════════════"
printf "  Results: ${GREEN}%d${NC}/%d passed, ${RED}%d${NC} failed\n" "$PASS" "$TOTAL" "$FAIL"
echo "═══════════════════════════════════════"
echo ""

if [[ "$FAIL" -gt 0 ]]; then
    echo "Stderr log: /tmp/neo_battery_stderr.log"
    exit 1
fi
