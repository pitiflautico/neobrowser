#!/bin/bash
# NeoRender V2 — Form Benchmark (E2E)
NEORENDER=target/release/neorender
PASS=0
TOTAL=0

test_site() {
    local name=$1 url=$2 action=$3 check=$4
    TOTAL=$((TOTAL + 1))
    echo -n "[$name] "
    # Run through interact REPL
    result=$(echo -e "$action\nquit" | timeout 30 $NEORENDER interact "$url" 2>&1)
    if echo "$result" | grep -q "$check"; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL (expected: $check)"
        echo "$result" | tail -5
    fi
}

# Build first
cargo build --release 2>/dev/null

# Test 1: example.com baseline
test_site "example.com" "https://example.com" "eval document.title" "Example Domain"

# Test 2: HN (data extraction)
test_site "hackernews" "https://news.ycombinator.com" "eval document.querySelectorAll('a').length" "links"

# Test 3: httpbin forms — fill and check value
test_site "httpbin-fill" "https://httpbin.org/forms/post" \
    "eval document.querySelector('input[name=custname]').value='Claude'; document.querySelector('input[name=custname]').value" \
    "Claude"

# Test 4: DuckDuckGo — type in search
test_site "ddg-type" "https://duckduckgo.com" \
    "type input[name=q] test query\neval document.querySelector('input[name=q]').value" \
    "test query"

# Test 5: DDG form submit triggers navigation
test_site "ddg-submit" "https://duckduckgo.com" \
    "type input[name=q] neorender\nsubmit form#searchbox_homepage_search\neval document.title" \
    "Navigation triggered"

echo ""
echo "=== Form Benchmark: $PASS/$TOTAL passed ==="
[ $PASS -ge 4 ] && echo "GATE: PASS" || echo "GATE: FAIL"
