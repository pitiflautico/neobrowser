#!/bin/bash
BIN="./target/release/neobrowser_rs"
PASS=0; FAIL=0; TOTAL=0

neo_fetch() {
    echo "$1" | timeout 10 "$BIN" mcp 2>/dev/null | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line.strip())
        if 'result' in d:
            for c in d['result']['content']:
                print(c['text'])
    except: pass
"
}

check() {
    local name="$1" result="$2" assertion="$3"
    TOTAL=$((TOTAL+1))
    local ok=$(echo "$result" | python3 -c "
import json,sys
try:
    d=json.loads(sys.stdin.read())
    $assertion
except Exception as e:
    print('FAIL: '+str(e))
")
    if [ "$ok" = "PASS" ]; then
        printf "  ✅ %-35s\n" "$name"
        PASS=$((PASS+1))
    else
        printf "  ❌ %-35s %s\n" "$name" "$ok"
        FAIL=$((FAIL+1))
    fi
}

echo "═══════════════════════════════════"
echo " NeoRender API Test Battery"
echo "═══════════════════════════════════"

# T1: GET JSON
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://jsonplaceholder.typicode.com/posts/1"}}}')
check "T1: GET JSON" "$R" "print('PASS' if d.get('status')==200 and 'userId' in d.get('body','') else 'FAIL: status='+str(d.get('status')))"

# T2: POST JSON
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://jsonplaceholder.typicode.com/posts","method":"POST","headers":{"Content-Type":"application/json"},"body":"{\"title\":\"test\",\"body\":\"hello\",\"userId\":1}"}}}')
check "T2: POST JSON" "$R" "print('PASS' if d.get('status')==201 else 'FAIL: status='+str(d.get('status')))"

# T3: Chrome UA headers
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://httpbin.org/headers"}}}')
check "T3: Chrome 136 User-Agent" "$R" "b=d.get('body',''); print('PASS' if 'Chrome/136' in b else 'FAIL: no Chrome 136 in headers')"

# T4: ChatGPT auth (skip if no cookies)
if [ -f /tmp/chatgpt-fresh.json ]; then
    R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://chatgpt.com/api/auth/session","cookies_file":"/tmp/chatgpt-fresh.json"}}}')
    check "T4: ChatGPT auth token" "$R" "print('PASS' if d.get('status')==200 and 'accessToken' in d.get('body','') else 'FAIL: status='+str(d.get('status')))"
else
    echo "  ⏭  T4: ChatGPT auth (SKIPPED — no cookies)"
fi

# T5: GitHub API
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://api.github.com/repos/anthropics/claude-code"}}}')
check "T5: GitHub API" "$R" "print('PASS' if d.get('status')==200 and 'full_name' in d.get('body','') else 'FAIL: status='+str(d.get('status')))"

# T6: Custom headers
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://httpbin.org/headers","headers":{"X-Custom":"neorender","Authorization":"Bearer test123"}}}}')
check "T6: Custom headers" "$R" "b=d.get('body',''); print('PASS' if 'neorender' in b.lower() or 'X-Custom' in b else 'FAIL: custom header missing')"

# T7: Redirect following
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://httpbin.org/redirect/2"}}}')
check "T7: Redirect following" "$R" "print('PASS' if d.get('status')==200 else 'FAIL: status='+str(d.get('status')))"

# T8: Error handling (404)
R=$(neo_fetch '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://httpbin.org/status/404"}}}')
check "T8: 404 error handling" "$R" "print('PASS' if d.get('status')==404 and d.get('ok')==False else 'FAIL: status='+str(d.get('status'))+' ok='+str(d.get('ok')))"

echo ""
echo "═══════════════════════════════════"
echo " Results: $PASS/$TOTAL passed, $FAIL failed"
echo "═══════════════════════════════════"
