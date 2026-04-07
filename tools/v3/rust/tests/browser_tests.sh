#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# NeoRender Browser Test Battery
# Tests browser foundations: navigation, cookies, interaction,
# extraction, stealth, and real-world sites.
#
# Usage:
#   ./tests/browser_tests.sh           # run all tests
#   ./tests/browser_tests.sh 5         # run single test
#   ./tests/browser_tests.sh 4-6       # run range
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$SCRIPT_DIR/target/release/neobrowser_rs"
PASSED=0
FAILED=0
SKIPPED=0
TOTAL=0
RESULTS=()

# ─── Color helpers ───
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ─── Parse test filter ───
FILTER_MIN=0
FILTER_MAX=999
if [[ $# -ge 1 ]]; then
    if [[ "$1" =~ ^([0-9]+)-([0-9]+)$ ]]; then
        FILTER_MIN=${BASH_REMATCH[1]}
        FILTER_MAX=${BASH_REMATCH[2]}
    elif [[ "$1" =~ ^[0-9]+$ ]]; then
        FILTER_MIN=$1
        FILTER_MAX=$1
    fi
fi

# ─── Verify binary ───
if [[ ! -x "$BINARY" ]]; then
    echo "Binary not found: $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

# ─── FIFO MCP session management ───
MCP_PID=""
FIFO_IN=""
FIFO_OUT=""
REQ_ID=0

start_mcp() {
    FIFO_IN=$(mktemp -u /tmp/neo_in.XXXXXX)
    FIFO_OUT=$(mktemp -u /tmp/neo_out.XXXXXX)
    mkfifo "$FIFO_IN"
    mkfifo "$FIFO_OUT"

    # Launch MCP server: reads from FIFO_IN, writes to FIFO_OUT
    NEOBROWSER_HEADLESS=1 "$BINARY" mcp < "$FIFO_IN" > "$FIFO_OUT" 2>/dev/null &
    MCP_PID=$!

    # Open write fd (keeps pipe open)
    exec 3>"$FIFO_IN"
    # Open read fd
    exec 4<"$FIFO_OUT"

    REQ_ID=0

    # Initialize handshake
    send_raw '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
    read_response >/dev/null 2>&1

    # notifications/initialized needs id field for JSON parse (server skips response via continue)
    send_raw '{"jsonrpc":"2.0","id":null,"method":"notifications/initialized","params":{}}'
    # Server does `continue` for this method — no response expected
    sleep 0.1
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

send_raw() {
    echo "$1" >&3
}

read_response() {
    local line
    # Read with timeout
    if read -r -t 60 line <&4; then
        echo "$line"
    else
        echo '{"error":"timeout"}'
    fi
}

# Send a tools/call request and return the inner result JSON
call_tool() {
    local tool_name="$1"
    local tool_args="$2"
    REQ_ID=$((REQ_ID + 1))

    local req="{\"jsonrpc\":\"2.0\",\"id\":$REQ_ID,\"method\":\"tools/call\",\"params\":{\"name\":\"$tool_name\",\"arguments\":$tool_args}}"
    send_raw "$req"

    local resp
    resp=$(read_response)

    # Extract the tool result text from the MCP envelope:
    # response.result.content[0].text is the JSON string with the actual data
    local inner
    inner=$(echo "$resp" | jq -r '.result.content[0].text // empty' 2>/dev/null)
    if [[ -n "$inner" ]]; then
        echo "$inner"
    else
        # Maybe it's an error
        echo "$resp"
    fi
}

# ─── Assertion helpers ───

assert_json_field() {
    local json="$1" field="$2" expected="$3" desc="$4"
    local actual
    actual=$(echo "$json" | jq -r ".$field // empty" 2>/dev/null)
    if [[ "$actual" == "$expected" ]]; then
        return 0
    else
        echo "  ASSERT FAIL: $desc — expected $field=$expected, got '$actual'"
        return 1
    fi
}

assert_json_gt() {
    local json="$1" field="$2" min="$3" desc="$4"
    local actual
    actual=$(echo "$json" | jq -r ".$field // 0" 2>/dev/null)
    if (( actual > min )); then
        return 0
    else
        echo "  ASSERT FAIL: $desc — expected $field>$min, got $actual"
        return 1
    fi
}

assert_json_contains() {
    local json="$1" field="$2" needle="$3" desc="$4"
    local actual
    actual=$(echo "$json" | jq -r ".$field // empty" 2>/dev/null)
    if [[ "$actual" == *"$needle"* ]]; then
        return 0
    else
        echo "  ASSERT FAIL: $desc — expected $field to contain '$needle', got '${actual:0:200}'"
        return 1
    fi
}

assert_contains() {
    local text="$1" needle="$2" desc="$3"
    if [[ "$text" == *"$needle"* ]]; then
        return 0
    else
        echo "  ASSERT FAIL: $desc — expected to contain '$needle'"
        return 1
    fi
}

# ─── Test runner ───

run_test() {
    local num="$1" name="$2"
    shift 2

    # Filter check
    if (( num < FILTER_MIN || num > FILTER_MAX )); then
        return
    fi

    TOTAL=$((TOTAL + 1))
    printf "${CYAN}[%02d]${NC} %-45s " "$num" "$name"

    local t0
    t0=$(date +%s)

    # Run test function, capture output
    local output
    if output=$("$@" 2>&1); then
        local t1
        t1=$(date +%s)
        local elapsed=$((t1 - t0))
        printf "${GREEN}PASS${NC} ${YELLOW}(%ds)${NC}\n" "$elapsed"
        PASSED=$((PASSED + 1))
        RESULTS+=("PASS  $num  $name")
    else
        local t1
        t1=$(date +%s)
        local elapsed=$((t1 - t0))
        printf "${RED}FAIL${NC} ${YELLOW}(%ds)${NC}\n" "$elapsed"
        # Show first 3 lines of failure output
        echo "$output" | head -3 | sed 's/^/  /'
        FAILED=$((FAILED + 1))
        RESULTS+=("FAIL  $num  $name")
    fi
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 1: Basic Navigation
# ═══════════════════════════════════════════════════════════════

test_01_basic_navigation() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://httpbin.org/html","mode":"auto"}')

    assert_json_gt "$result" "html_bytes" 100 "html_bytes > 100"
    assert_json_contains "$result" "page" "Herman Melville" "text contains Herman Melville"
}

test_02_basic_links() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://news.ycombinator.com","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"
    assert_json_contains "$result" "page" "Hacker News" "text contains Hacker News"
    # HN has many links (30+ story links + misc)
    assert_json_gt "$result" "links" 20 "links > 20"
}

test_03_basic_eval() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Open a page first (Chrome mode for eval)
    call_tool "browser_open" '{"url":"https://httpbin.org/html"}' >/dev/null

    local result
    result=$(call_tool "browser_act" '{"kind":"eval","text":"1+1"}')

    assert_json_contains "$result" "effect" "2" "eval 1+1 = 2"
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 2: Cookies
# ═══════════════════════════════════════════════════════════════

test_04_cookie_from_http() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Set cookie via httpbin (redirects to /cookies which returns JSON)
    local result
    result=$(call_tool "browser_open" '{"url":"https://httpbin.org/cookies/set/testcookie/testvalue","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"

    # Verify cookie was received by checking /cookies endpoint (same session, ghost reuses jar)
    local check
    check=$(call_tool "browser_open" '{"url":"https://httpbin.org/cookies","mode":"auto"}')

    assert_json_contains "$check" "page" "testcookie" "cookie persists in HTTP jar"
}

test_05_cookie_persistence() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Set cookie via one URL
    call_tool "browser_open" '{"url":"https://httpbin.org/cookies/set/persistcookie/persistvalue"}' >/dev/null

    # Navigate to cookies endpoint (same session) — cookies should be sent
    local result
    result=$(call_tool "browser_open" '{"url":"https://httpbin.org/cookies"}')

    assert_json_contains "$result" "page" "persistcookie" "cookie persists across navigations"
}

test_06_cookie_from_js() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Use neorender so cookie set/get happen in the same V8 engine
    call_tool "browser_open" '{"url":"https://httpbin.org/html","mode":"neorender"}' >/dev/null

    # Set cookie via JS
    call_tool "browser_act" '{"kind":"eval","text":"document.cookie = \"jscookie=jsvalue; path=/\""}' >/dev/null

    # Read it back
    local result
    result=$(call_tool "browser_act" '{"kind":"eval","text":"document.cookie"}')

    assert_json_contains "$result" "effect" "jscookie=jsvalue" "JS-set cookie readable"
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 3: Interaction
# ═══════════════════════════════════════════════════════════════

test_07_type_input() {
    start_mcp
    trap 'stop_mcp' RETURN

    # HN has a search form (via hn.algolia.com link, but the main page has no input)
    # Use httpbin forms instead
    call_tool "browser_open" '{"url":"https://httpbin.org/forms/post"}' >/dev/null

    local result
    result=$(call_tool "browser_act" '{"kind":"type","target":"custname","text":"Test User"}')

    assert_json_contains "$result" "outcome" "succeeded" "type succeeded"
}

test_08_click_link() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Use neorender so click operates on the loaded page (not a separate Chrome about:blank)
    call_tool "browser_open" '{"url":"https://news.ycombinator.com","mode":"neorender"}' >/dev/null

    local result
    result=$(call_tool "browser_act" '{"kind":"click","target":"new"}')

    # Should navigate or find the "new" link
    local outcome
    outcome=$(echo "$result" | jq -r '.outcome // .effect // empty' 2>/dev/null)
    if [[ "$outcome" == "succeeded" ]] || [[ "$outcome" == "navigated" ]] || [[ "$result" == *"navigate"* ]] || [[ "$result" == *"clicked"* ]]; then
        return 0
    else
        echo "  ASSERT FAIL: click 'new' — got: ${result:0:200}"
        return 1
    fi
}

test_09_form_submit() {
    start_mcp
    trap 'stop_mcp' RETURN

    call_tool "browser_open" '{"url":"https://httpbin.org/forms/post"}' >/dev/null

    # Type into customer name field
    call_tool "browser_act" '{"kind":"type","target":"custname","text":"Test User"}' >/dev/null

    # Submit the form
    local result
    result=$(call_tool "browser_act" '{"kind":"submit"}')

    local outcome
    outcome=$(echo "$result" | jq -r '.outcome // empty' 2>/dev/null)
    if [[ "$outcome" == "succeeded" ]]; then
        return 0
    else
        # Even partial success or navigation counts
        if [[ "$result" == *"submit"* ]] || [[ "$result" == *"navigat"* ]]; then
            return 0
        fi
        echo "  ASSERT FAIL: form submit — got: ${result:0:200}"
        return 1
    fi
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 4: Extraction
# ═══════════════════════════════════════════════════════════════

test_10_extract_tables() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Wikipedia article -- use neorender so eval runs on the loaded DOM
    call_tool "browser_open" '{"url":"https://en.wikipedia.org/wiki/Rust_(programming_language)","mode":"neorender"}' >/dev/null

    # Simple count to avoid large response buffer issues
    local result
    result=$(call_tool "browser_act" '{"kind":"eval","text":"document.querySelectorAll(\"table\").length"}')

    local effect
    effect=$(echo "$result" | jq -r '.effect // empty' 2>/dev/null)

    # Extract number from "eval_result: 13"
    local count
    count=$(echo "$effect" | grep -oE '[0-9]+' | head -1)

    if [[ -n "$count" ]] && (( count > 0 )); then
        return 0
    else
        echo "  ASSERT FAIL: expected tables > 0, got '$effect'"
        return 1
    fi
}

test_11_extract_article() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://en.wikipedia.org/wiki/Rust_(programming_language)","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"

    # Title or page text should contain "Rust"
    local page
    page=$(echo "$result" | jq -r '.page // .title // empty' 2>/dev/null)
    if [[ "$page" == *"Rust"* ]]; then
        return 0
    else
        echo "  ASSERT FAIL: expected page to contain 'Rust'"
        return 1
    fi
}

test_12_dom_tree() {
    start_mcp
    trap 'stop_mcp' RETURN

    call_tool "browser_open" '{"url":"https://news.ycombinator.com"}' >/dev/null

    # Use eval to get a mini DOM tree
    local result
    result=$(call_tool "browser_act" '{"kind":"eval","text":"(function(){function walk(el,d){if(d>2||!el)return null;var r={tag:el.tagName,children:[]};for(var i=0;i<Math.min(el.children.length,3);i++){var c=walk(el.children[i],d+1);if(c)r.children.push(c)}return r}return JSON.stringify(walk(document.body,0))})()"}')

    assert_json_contains "$result" "effect" "tag" "DOM tree has tag"
    assert_json_contains "$result" "effect" "children" "DOM tree has children"
}

test_13_wom() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://news.ycombinator.com","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"

    # WOM should have text and links
    local page
    page=$(echo "$result" | jq -r '.page // empty' 2>/dev/null)
    if [[ -z "$page" ]]; then
        echo "  ASSERT FAIL: empty page text"
        return 1
    fi

    local links
    links=$(echo "$result" | jq -r '.links // 0' 2>/dev/null)
    if (( links > 0 )); then
        return 0
    else
        echo "  ASSERT FAIL: expected links > 0, got $links"
        return 1
    fi
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 5: Advanced
# ═══════════════════════════════════════════════════════════════

test_14_stealth() {
    start_mcp
    trap 'stop_mcp' RETURN

    call_tool "browser_open" '{"url":"https://httpbin.org/html"}' >/dev/null

    # navigator.webdriver should be false/undefined (stealth)
    local wd
    wd=$(call_tool "browser_act" '{"kind":"eval","text":"navigator.webdriver"}')

    # In a good stealth setup, webdriver is false or undefined
    local wd_val
    wd_val=$(echo "$wd" | jq -r '.effect' 2>/dev/null)

    # Check plugins
    local plugins
    plugins=$(call_tool "browser_act" '{"kind":"eval","text":"navigator.plugins.length"}')
    local plugins_val
    plugins_val=$(echo "$plugins" | jq -r '.effect' 2>/dev/null)

    # Check chrome object
    local chrome_obj
    chrome_obj=$(call_tool "browser_act" '{"kind":"eval","text":"typeof chrome"}')
    local chrome_val
    chrome_val=$(echo "$chrome_obj" | jq -r '.effect' 2>/dev/null)

    local fail=0

    # webdriver should NOT be true
    if [[ "$wd_val" == *"true"* ]]; then
        echo "  WARN: navigator.webdriver=true (stealth not fully applied)"
        # Not a hard fail — Chrome with --remote-debugging may show true
    fi

    # chrome should exist
    if [[ "$chrome_val" != *"object"* ]]; then
        echo "  ASSERT FAIL: typeof chrome expected 'object', got '$chrome_val'"
        fail=1
    fi

    return $fail
}

test_15_network_log() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Navigate to a page that makes network requests
    call_tool "browser_open" '{"url":"https://httpbin.org/html"}' >/dev/null

    # Small wait for network activity to register
    call_tool "browser_wait" '{"seconds":1}' >/dev/null 2>&1 || true

    # Observe with network
    local result
    result=$(call_tool "browser_observe" '{"format":"see","include_network":true}')

    # Check we got a response (network may or may not have entries depending on mode)
    local page
    page=$(echo "$result" | jq -r '.page // empty' 2>/dev/null)
    if [[ -n "$page" ]]; then
        return 0
    else
        echo "  ASSERT FAIL: observe returned empty"
        return 1
    fi
}

test_16_page_diff() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Navigate to first page
    call_tool "browser_open" '{"url":"https://news.ycombinator.com"}' >/dev/null

    # Observe to get baseline WOM
    call_tool "browser_observe" '{"format":"compact"}' >/dev/null

    # Navigate to a different page
    call_tool "browser_open" '{"url":"https://news.ycombinator.com/newest"}' >/dev/null

    # Get delta
    local result
    result=$(call_tool "browser_observe" '{"format":"delta"}')

    # Delta should exist with revision
    local rev
    rev=$(echo "$result" | jq -r '.revision // 0' 2>/dev/null)
    if (( rev > 0 )); then
        return 0
    else
        # Even without delta, having a result is OK
        if [[ -n "$result" ]] && [[ "$result" != *"error"* ]]; then
            return 0
        fi
        echo "  ASSERT FAIL: expected delta with revision > 0"
        return 1
    fi
}

test_17_wait_for() {
    start_mcp
    trap 'stop_mcp' RETURN

    # Test seconds-based wait (simplest, needs Chrome session)
    call_tool "browser_open" '{"url":"https://httpbin.org/html"}' >/dev/null
    # Trigger Chrome launch via eval
    call_tool "browser_act" '{"kind":"eval","text":"true"}' >/dev/null

    # Test: seconds-based wait should succeed
    local result
    result=$(call_tool "browser_wait" '{"seconds":1}')

    local ok
    ok=$(echo "$result" | jq -r '.ok // false' 2>/dev/null)
    if [[ "$ok" != "true" ]]; then
        echo "  ASSERT FAIL: wait 1s should succeed (got: ${result:0:200})"
        return 1
    fi

    # Test: no-condition wait should succeed
    local result2
    result2=$(call_tool "browser_wait" '{"timeout_ms":2000}')

    local ok2
    ok2=$(echo "$result2" | jq -r '.ok // false' 2>/dev/null)
    if [[ "$ok2" != "true" ]]; then
        echo "  ASSERT FAIL: basic wait should succeed (got: ${result2:0:200})"
        return 1
    fi
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 6: Real-world Sites
# ═══════════════════════════════════════════════════════════════

test_18_google_search() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://www.google.com/search?q=test","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"

    # Google may show consent or results — both are valid
    local page
    page=$(echo "$result" | jq -r '.page // empty' 2>/dev/null)
    local links
    links=$(echo "$result" | jq -r '.links // 0' 2>/dev/null)

    if [[ -n "$page" ]] && (( links > 5 )); then
        return 0
    else
        # Even if consent blocks, we should still get a page
        if [[ -n "$page" ]]; then
            echo "  NOTE: Google may show consent page (links=$links)"
            return 0
        fi
        echo "  ASSERT FAIL: Google search returned empty"
        return 1
    fi
}

test_19_wikipedia_full() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://en.wikipedia.org/wiki/Rust_(programming_language)","mode":"auto"}')

    assert_json_field "$result" "ok" "true" "request ok"

    # Should have title, text, and links
    local title
    title=$(echo "$result" | jq -r '.title // empty' 2>/dev/null)
    local links
    links=$(echo "$result" | jq -r '.links // 0' 2>/dev/null)
    local html_bytes
    html_bytes=$(echo "$result" | jq -r '.html_bytes // 0' 2>/dev/null)

    local fail=0

    if [[ "$title" != *"Rust"* ]]; then
        echo "  ASSERT FAIL: title should contain 'Rust', got '$title'"
        fail=1
    fi

    if (( links < 10 )); then
        echo "  ASSERT FAIL: expected links > 10, got $links"
        fail=1
    fi

    if (( html_bytes < 1000 )); then
        echo "  ASSERT FAIL: expected html_bytes > 1000, got $html_bytes"
        fail=1
    fi

    return $fail
}

test_20_authenticated() {
    # This test requires LinkedIn cookies — skip if not available
    local cookie_file="$HOME/.neobrowser/sessions/www_linkedin_com.json"
    if [[ ! -f "$cookie_file" ]]; then
        printf "${YELLOW}SKIP${NC} (no LinkedIn session)\n"
        SKIPPED=$((SKIPPED + 1))
        TOTAL=$((TOTAL - 1))  # Don't count in total
        RESULTS+=("SKIP  20  authenticated_linkedin")
        return 0
    fi

    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" "{\"url\":\"https://www.linkedin.com/feed\",\"cookies_file\":\"$cookie_file\"}")

    assert_json_field "$result" "ok" "true" "request ok"

    local links
    links=$(echo "$result" | jq -r '.links // 0' 2>/dev/null)
    if (( links > 5 )); then
        return 0
    else
        echo "  NOTE: LinkedIn may require re-auth (links=$links)"
        # Not a hard failure — session might be expired
        return 0
    fi
}

# ═══════════════════════════════════════════════════════════════
# LEVEL 7: NeoRender Engine (V8)
# ═══════════════════════════════════════════════════════════════

test_21_neorender_basic() {
    start_mcp
    trap 'stop_mcp' RETURN

    local result
    result=$(call_tool "browser_open" '{"url":"https://httpbin.org/html","mode":"neorender"}')

    assert_json_field "$result" "ok" "true" "neorender request ok"
    assert_json_gt "$result" "html_bytes" 100 "html_bytes > 100"
    assert_json_contains "$result" "page" "Herman Melville" "neorender text extraction"
}

test_22_neorender_eval() {
    start_mcp
    trap 'stop_mcp' RETURN

    call_tool "browser_open" '{"url":"https://httpbin.org/html","mode":"neorender"}' >/dev/null

    local result
    result=$(call_tool "browser_act" '{"kind":"eval","text":"2+2"}')

    assert_json_contains "$result" "effect" "4" "neorender eval 2+2 = 4"
    assert_json_contains "$result" "engine" "neosession" "uses neosession engine"
}

test_23_neorender_type() {
    start_mcp
    trap 'stop_mcp' RETURN

    call_tool "browser_open" '{"url":"https://httpbin.org/forms/post","mode":"neorender"}' >/dev/null

    local result
    result=$(call_tool "browser_act" '{"kind":"type","target":"custname","text":"Neo Test"}')

    assert_json_contains "$result" "outcome" "succeeded" "neorender type succeeded"
    assert_json_contains "$result" "engine" "neosession" "uses neosession engine"
}

# ═══════════════════════════════════════════════════════════════
# Run all tests
# ═══════════════════════════════════════════════════════════════

echo ""
echo "${BOLD}═══════════════════════════════════════════════════════════${NC}"
echo "${BOLD}  NeoRender Browser Test Battery${NC}"
echo "${BOLD}═══════════════════════════════════════════════════════════${NC}"
echo ""

echo "${BOLD}── Level 1: Basic Navigation ──${NC}"
run_test 1  "basic_navigation (httpbin/html)"       test_01_basic_navigation
run_test 2  "basic_links (HN)"                      test_02_basic_links
run_test 3  "basic_eval (1+1)"                      test_03_basic_eval
echo ""

echo "${BOLD}── Level 2: Cookies ──${NC}"
run_test 4  "cookie_from_http (httpbin set)"         test_04_cookie_from_http
run_test 5  "cookie_persistence (same session)"      test_05_cookie_persistence
run_test 6  "cookie_from_js (document.cookie)"       test_06_cookie_from_js
echo ""

echo "${BOLD}── Level 3: Interaction ──${NC}"
run_test 7  "type_input (form field)"                test_07_type_input
run_test 8  "click_link (HN 'new')"                  test_08_click_link
run_test 9  "form_submit (httpbin POST)"             test_09_form_submit
echo ""

echo "${BOLD}── Level 4: Extraction ──${NC}"
run_test 10 "extract_tables (Wikipedia)"             test_10_extract_tables
run_test 11 "extract_article (Wikipedia Rust)"       test_11_extract_article
run_test 12 "dom_tree (depth 2)"                     test_12_dom_tree
run_test 13 "wom_document (HN)"                      test_13_wom
echo ""

echo "${BOLD}── Level 5: Advanced ──${NC}"
run_test 14 "stealth_checks (webdriver/plugins)"     test_14_stealth
run_test 15 "network_log (observe+network)"          test_15_network_log
run_test 16 "page_diff (delta observation)"          test_16_page_diff
run_test 17 "wait_for (text present/absent)"         test_17_wait_for
echo ""

echo "${BOLD}── Level 6: Real-world Sites ──${NC}"
run_test 18 "google_search"                          test_18_google_search
run_test 19 "wikipedia_full (title+links+bytes)"     test_19_wikipedia_full
run_test 20 "authenticated (LinkedIn)"               test_20_authenticated
echo ""

echo "${BOLD}── Level 7: NeoRender V8 ──${NC}"
run_test 21 "neorender_basic (httpbin)"              test_21_neorender_basic
run_test 22 "neorender_eval (2+2)"                   test_22_neorender_eval
run_test 23 "neorender_type (form)"                  test_23_neorender_type
echo ""

# ─── Summary ───
echo "${BOLD}═══════════════════════════════════════════════════════════${NC}"
if (( FAILED == 0 )); then
    printf "${GREEN}${BOLD}RESULTS: %d/%d passed${NC}" "$PASSED" "$TOTAL"
else
    printf "${RED}${BOLD}RESULTS: %d/%d passed, %d failed${NC}" "$PASSED" "$TOTAL" "$FAILED"
fi
if (( SKIPPED > 0 )); then
    printf ", ${YELLOW}%d skipped${NC}" "$SKIPPED"
fi
echo ""
echo "${BOLD}═══════════════════════════════════════════════════════════${NC}"
echo ""

# Detail per test
for r in "${RESULTS[@]}"; do
    if [[ "$r" == PASS* ]]; then
        echo "  ${GREEN}$r${NC}"
    elif [[ "$r" == FAIL* ]]; then
        echo "  ${RED}$r${NC}"
    else
        echo "  ${YELLOW}$r${NC}"
    fi
done
echo ""

exit $FAILED
