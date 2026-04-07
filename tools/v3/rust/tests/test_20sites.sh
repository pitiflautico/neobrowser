#!/bin/bash
# Test: 20 top sites via NeoRender (V8 browser, no Chrome)
# Pass criteria: 18+ sites load without WAF block
BIN="./target/release/neobrowser_rs"

SITES=(
    "https://news.ycombinator.com"
    "https://www.google.es/search?q=test"
    "https://www.reddit.com"
    "https://www.youtube.com"
    "https://en.wikipedia.org/wiki/Spain"
    "https://www.amazon.es"
    "https://stackoverflow.com/questions"
    "https://chatgpt.com"
    "https://www.nytimes.com"
    "https://www.bbc.com"
    "https://www.elpais.com"
    "https://www.apple.com"
    "https://www.microsoft.com"
    "https://www.netflix.com"
    "https://www.instagram.com"
    "https://www.notion.so"
    "https://docs.google.com"
    "https://www.twitch.tv"
    "https://www.facebook.com"
    "https://www.linkedin.com/feed"
)

PASS=0
FAIL=0
TIMEOUT=0

echo "═══════════════════════════════════════"
echo " NeoRender 20-site test (Chrome 136 TLS)"
echo "═══════════════════════════════════════"

for url in "${SITES[@]}"; do
    COOKIES=""
    case "$url" in
        *linkedin*) COOKIES=',"cookies_file":"/tmp/linkedin-fresh.json"';;
        *amazon*) COOKIES=',"cookies_file":"/tmp/amazon-state.json"';;
        *chatgpt*) COOKIES=',"cookies_file":"/tmp/chatgpt-fresh.json"';;
    esac

    result=$(echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"browser_open\",\"arguments\":{\"url\":\"$url\",\"mode\":\"neorender\"$COOKIES}}}" | timeout 30 "$BIN" mcp 2>/dev/null | python3 -c "
import json,sys
for line in sys.stdin:
    try:
        d=json.loads(line.strip())
        if 'result' in d:
            for c in d['result']['content']:
                data=json.loads(c['text'])
                ok=data.get('ok',False)
                kb=data.get('html_bytes',0)//1024
                links=data.get('links',0)
                blocked=data.get('blocked','')
                ptype=data.get('page_type','')
                if blocked: print(f'BLOCK {blocked}')
                elif ok and kb>10: print(f'OK {kb}KB L:{links} [{ptype}]')
                elif ok: print(f'WEAK {kb}KB')
                else: print('FAIL')
    except: pass
" 2>/dev/null)

    site=$(echo "$url" | sed 's|https://||;s|/.*||;s|www\.||')

    if [ -z "$result" ]; then
        printf "  ⏱ %-22s timeout\n" "$site"
        TIMEOUT=$((TIMEOUT+1))
    elif echo "$result" | grep -q "^OK"; then
        printf "  ✅ %-22s %s\n" "$site" "$result"
        PASS=$((PASS+1))
    elif echo "$result" | grep -q "^BLOCK"; then
        printf "  🚫 %-22s %s\n" "$site" "$result"
        FAIL=$((FAIL+1))
    else
        printf "  ⚠️  %-22s %s\n" "$site" "$result"
        FAIL=$((FAIL+1))
    fi
done

echo ""
echo "═══════════════════════════════════════"
echo " Results: $PASS/20 ✅  $FAIL ❌  $TIMEOUT ⏱"
echo "═══════════════════════════════════════"

if [ $PASS -ge 18 ]; then
    echo " ✅ PASS (18+ sites required)"
else
    echo " ❌ FAIL (only $PASS/18 required)"
fi
