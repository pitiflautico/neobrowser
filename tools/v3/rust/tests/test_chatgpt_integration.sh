#!/bin/bash
# Integration test: ChatGPT loads + hydrates + API accessible via NeoRender
BIN="./target/release/neobrowser_rs"
COOKIES="/tmp/chatgpt-fresh.json"

cd "$(dirname "$0")/.."

# Check cookies exist
if [ ! -f "$COOKIES" ]; then
    echo "Missing cookies: $COOKIES"
    echo "   Export from Chrome first"
    exit 1
fi

echo "==============================="
echo " ChatGPT Integration Test"
echo "==============================="

# Test 1: Load page + check hydration
echo ""
echo "> Test 1: Load + Hydrate..."
RESULT=$(timeout 15 "$BIN" mcp 2>/dev/null << 'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"https://chatgpt.com","mode":"neorender","cookies_file":"/tmp/chatgpt-fresh.json"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"browser_act","arguments":{"kind":"eval","text":"JSON.stringify({routeModules:!!window.__reactRouterRouteModules,nodes:document.querySelectorAll('*').length,links:document.querySelectorAll('a').length,buttons:document.querySelectorAll('button').length})"}}}
EOF
)

# Parse results
HYDRATED=$(echo "$RESULT" | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if d.get('id')==2 and 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                eff=data.get('effect','')
                if 'eval_result:' in eff:
                    r=json.loads(eff[len('eval_result: '):])
                    print('true' if r.get('routeModules') else 'false')
    except: pass
" 2>/dev/null)

RENDER_MS=$(echo "$RESULT" | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if d.get('id')==1 and 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                print(data.get('render_ms',0))
    except: pass
" 2>/dev/null)

NODE_COUNT=$(echo "$RESULT" | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if d.get('id')==2 and 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                eff=data.get('effect','')
                if 'eval_result:' in eff:
                    r=json.loads(eff[len('eval_result: '):])
                    print(r.get('nodes',0))
    except: pass
" 2>/dev/null)

if [ "$HYDRATED" = "true" ]; then
    echo "  PASS Hydrated (${RENDER_MS}ms)"
else
    echo "  FAIL Not hydrated"
fi

# Test 2: Auth token via browser_fetch
echo ""
echo "> Test 2: Auth Token..."
AUTH_RESULT=$(timeout 10 "$BIN" mcp 2>/dev/null << 'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://chatgpt.com/api/auth/session","cookies_file":"/tmp/chatgpt-fresh.json"}}}
EOF
)

AUTH_TOKEN=$(echo "$AUTH_RESULT" | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                body=json.loads(data.get('body','{}'))
                print(body.get('accessToken',''))
    except: pass
" 2>/dev/null)

if [ -n "$AUTH_TOKEN" ] && [ ${#AUTH_TOKEN} -gt 100 ]; then
    AUTH_OK="true"
    echo "  PASS Auth token obtained (${#AUTH_TOKEN} chars)"
else
    AUTH_OK="false"
    echo "  FAIL Auth failed (token=${#AUTH_TOKEN} chars)"
fi

# Test 3: Sentinel token
echo ""
echo "> Test 3: Sentinel Token..."
if [ -n "$AUTH_TOKEN" ] && [ ${#AUTH_TOKEN} -gt 100 ]; then
    SENTINEL_RESULT=$(timeout 10 "$BIN" mcp 2>/dev/null << EOF
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_fetch","arguments":{"url":"https://chatgpt.com/backend-api/sentinel/chat-requirements","method":"POST","headers":{"Authorization":"Bearer $AUTH_TOKEN","Content-Type":"application/json"},"body":"{}","cookies_file":"/tmp/chatgpt-fresh.json"}}}
EOF
)

    SENTINEL_OK=$(echo "$SENTINEL_RESULT" | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                body=json.loads(data.get('body','{}'))
                token=body.get('token','')
                if len(token) > 10:
                    print('true')
                    break
                else:
                    print('false')
                    break
    except: pass
" 2>/dev/null)

    if [ "$SENTINEL_OK" = "true" ]; then
        echo "  PASS Sentinel token obtained"
    else
        echo "  FAIL Sentinel request failed"
    fi
else
    SENTINEL_OK="false"
    echo "  FAIL No auth token for sentinel"
fi

# Test 4: Page content readable (from Test 1 session -- nodes rendered)
echo ""
echo "> Test 4: Page Content..."
if [ -n "$NODE_COUNT" ] && [ "$NODE_COUNT" -gt 10 ] 2>/dev/null; then
    CONTENT_OK="true"
    echo "  PASS DOM rendered ($NODE_COUNT nodes)"
else
    CONTENT_OK="false"
    echo "  FAIL DOM empty or too small (nodes=$NODE_COUNT)"
fi

# Summary
echo ""
echo "==============================="
PASS=0
TOTAL=4
[ "$HYDRATED" = "true" ] && PASS=$((PASS+1))
[ "$AUTH_OK" = "true" ] && PASS=$((PASS+1))
[ "$SENTINEL_OK" = "true" ] && PASS=$((PASS+1))
[ "$CONTENT_OK" = "true" ] && PASS=$((PASS+1))
echo " Result: $PASS/$TOTAL passed"
if [ $PASS -eq $TOTAL ]; then
    echo " ALL PASS -- ChatGPT works in NeoRender"
else
    echo " FAIL ($((TOTAL-PASS)) tests failed)"
fi
echo "==============================="
exit $(( TOTAL - PASS ))
