#!/bin/bash
# Test: ChatGPT loads identically with and without headless
# Pass criteria: both modes get same page_class, similar node count, no WAF block
BIN="./target/release/neobrowser_rs"
URL="https://chatgpt.com"
PIPELINE='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"'"$URL"'","mode":"chrome"}}}'

extract() {
    python3 -c "
import json,sys
for line in sys.stdin:
    d=json.loads(line.strip())
    if 'result' in d:
        for c in d['result']['content']:
            data=json.loads(c['text'])
            print(json.dumps({
                'ok': data.get('ok'),
                'engine': data.get('engine'),
                'page_class': data.get('page_class'),
                'nodes': data.get('nodes',0),
                'actions': data.get('actions',0),
                'blocked': data.get('blocked',''),
                'bytes': len(data.get('content',''))
            }))
"
}

echo "═══════════════════════════════════"
echo " Test: headless parity — $URL"
echo "═══════════════════════════════════"

# 1. Headed (visible)
echo ""
echo "▶ HEADED (visible Chrome)..."
pkill -9 -f "Chrome.*neobrowser" 2>/dev/null; sleep 1
HEADED=$(echo "$PIPELINE" | NEOBROWSER_HEADLESS=0 timeout 45 "$BIN" mcp 2>/dev/null | extract)
pkill -9 -f "Chrome.*neobrowser" 2>/dev/null; sleep 2
echo "  $HEADED"

# 2. Headless (offscreen)
echo ""
echo "▶ HEADLESS (offscreen Chrome)..."
HEADLESS=$(echo "$PIPELINE" | NEOBROWSER_HEADLESS=1 timeout 45 "$BIN" mcp 2>/dev/null | extract)
pkill -9 -f "Chrome.*neobrowser" 2>/dev/null; sleep 1
echo "  $HEADLESS"

# 3. Compare
echo ""
echo "═══════════════════════════════════"
echo " Results"
echo "═══════════════════════════════════"

H_CLASS=$(echo "$HEADED" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('page_class',''))")
L_CLASS=$(echo "$HEADLESS" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('page_class',''))")
H_NODES=$(echo "$HEADED" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('nodes',0))")
L_NODES=$(echo "$HEADLESS" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('nodes',0))")
H_BLOCK=$(echo "$HEADED" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('blocked',''))")
L_BLOCK=$(echo "$HEADLESS" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('blocked',''))")
H_OK=$(echo "$HEADED" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('ok',False))")
L_OK=$(echo "$HEADLESS" | python3 -c "import json,sys; print(json.loads(sys.stdin.read()).get('ok',False))")

PASS=true

# Check: both OK
if [ "$H_OK" = "True" ] && [ "$L_OK" = "True" ]; then
    echo "  ✅ Both loaded OK"
else
    echo "  ❌ Load failed — headed=$H_OK headless=$L_OK"
    PASS=false
fi

# Check: same page class
if [ "$H_CLASS" = "$L_CLASS" ]; then
    echo "  ✅ Same page_class: $H_CLASS"
else
    echo "  ❌ page_class differs — headed=$H_CLASS headless=$L_CLASS"
    PASS=false
fi

# Check: no WAF block
if [ -z "$H_BLOCK" ] && [ -z "$L_BLOCK" ]; then
    echo "  ✅ No WAF blocks"
elif [ -n "$L_BLOCK" ]; then
    echo "  ❌ Headless blocked: $L_BLOCK"
    PASS=false
else
    echo "  ⚠️  Headed blocked: $H_BLOCK"
fi

# Check: similar node count (headless should have >= 50% of headed)
if [ "$H_NODES" -gt 0 ] 2>/dev/null; then
    RATIO=$((L_NODES * 100 / H_NODES))
    if [ "$RATIO" -ge 50 ]; then
        echo "  ✅ Node parity: headed=$H_NODES headless=$L_NODES (${RATIO}%)"
    else
        echo "  ❌ Node mismatch: headed=$H_NODES headless=$L_NODES (${RATIO}%)"
        PASS=false
    fi
else
    echo "  ⚠️  Headed has 0 nodes — headless=$L_NODES"
fi

echo ""
if $PASS; then
    echo "  ✅ PASS — headless parity confirmed"
else
    echo "  ❌ FAIL — headless differs from headed"
fi
echo ""
