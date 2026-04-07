#!/bin/bash
# Test: React Router streaming SSR hydration in NeoRender
# Mimics ChatGPT's exact patterns: TLA, Promise.allSettled, getAll, pipeThrough
BIN="./target/release/neobrowser_rs"
DIR="tests/hydration-test"

cd "$(dirname "$0")/.."

# Start local server
python3 -m http.server 8765 -d "$DIR" &
HTTP_PID=$!
sleep 1

RESULT=$(timeout 15 "$BIN" mcp << 'EOF' 2>/dev/null | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line)
        if d.get('id')==2 and 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                eff=data.get('effect','')
                if 'eval_result:' in eff:
                    print(eff[len('eval_result: '):])
    except: pass
"
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"http://localhost:8765/index.html","mode":"neorender"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"browser_act","arguments":{"kind":"eval","text":"JSON.stringify({routeModules:!!window.__reactRouterRouteModules,oaiLogHTML:typeof window.__oai_logHTML,hydrationComplete:!!window.__neo_hydration_complete,routerVersion:window.__reactRouterVersion})"}}}
EOF
)

kill $HTTP_PID 2>/dev/null

echo "═══════════════════════════════"
echo " React Hydration Test"
echo "═══════════════════════════════"

PASS=true
for check in routeModules oaiLogHTML hydrationComplete routerVersion; do
    val=$(echo "$RESULT" | python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('$check','MISSING'))")
    if [ "$val" = "true" ] || [ "$val" = "function" ] || [ "$val" = "vendor-2.0" ]; then
        echo "  ✅ $check: $val"
    else
        echo "  ❌ $check: $val"
        PASS=false
    fi
done

echo ""
if $PASS; then echo "  ✅ PASS"; else echo "  ❌ FAIL"; fi
