#!/bin/bash
# R2.7 Parity Harness — Run V1 and V2 against same URL, compare outputs.
# Usage: ./run_parity.sh [url] [tolerance_percent]
set -euo pipefail

V1_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neobrowser-rs/target/release/neobrowser_rs"
V2_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neorender-v2/target/release/neorender"
URL="${1:-https://news.ycombinator.com}"
TOLERANCE="${2:-10}"

# -- Preflight --
for bin in "$V1_BIN" "$V2_BIN"; do
  if [ ! -x "$bin" ]; then
    echo "FATAL: binary not found: $bin" >&2
    exit 1
  fi
done

echo "=== Parity: $URL (tolerance ${TOLERANCE}%) ==="

# -- Run V1 (MCP JSON-RPC) --
V1_OUT=$(echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"'"$URL"'","mode":"neorender"}}}' \
  | timeout 30 "$V1_BIN" mcp 2>/dev/null \
  | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        d = json.loads(line)
        if 'result' in d:
            for c in d['result'].get('content', []):
                data = json.loads(c['text'])
                print(json.dumps({
                    'title': data.get('title', ''),
                    'links': data.get('links', 0),
                    'forms': data.get('forms', 0),
                    'buttons': data.get('buttons', 0),
                    'inputs': data.get('inputs', 0),
                    'page_type': data.get('page_type', ''),
                    'render_ms': data.get('render_ms', 0),
                    'scripts': data.get('scripts', 0),
                }))
                break
    except:
        pass
" 2>/dev/null || echo '{}')

# -- Run V2 (CLI see) --
V2_OUT=$(timeout 30 "$V2_BIN" see "$URL" 2>/dev/null | python3 -c "
import json, sys
try:
    d = json.loads(sys.stdin.read())
    wom = d.get('wom', {})
    nodes = wom.get('nodes', [])
    links = len([n for n in nodes if 'navigate' in n.get('actions', [])])
    buttons = len([n for n in nodes if n.get('role') == 'button'])
    inputs = len([n for n in nodes if n.get('role') == 'input'])
    forms = len([n for n in nodes if n.get('role') == 'form'])
    print(json.dumps({
        'title': d.get('title', ''),
        'links': links,
        'forms': forms,
        'buttons': buttons,
        'inputs': inputs,
        'page_type': wom.get('page_type', ''),
        'render_ms': d.get('render_ms', 0),
        'wom_nodes': len(nodes),
    }))
except Exception as e:
    print('{}')
" 2>/dev/null || echo '{}')

# -- Compare --
python3 << PYEOF
import json, sys

v1_raw = '''$V1_OUT'''
v2_raw = '''$V2_OUT'''
v1 = json.loads(v1_raw) if v1_raw.strip() else {}
v2 = json.loads(v2_raw) if v2_raw.strip() else {}
tolerance = $TOLERANCE / 100.0

if not v1:
    print("WARN: V1 returned no data")
if not v2:
    print("WARN: V2 returned no data")

critical = []
cosmetic = []

for key in ['title', 'links', 'forms', 'buttons', 'inputs', 'page_type']:
    v1v = v1.get(key, '')
    v2v = v2.get(key, '')

    if isinstance(v1v, int) and isinstance(v2v, int):
        if v1v == 0 and v2v == 0:
            continue
        base = max(v1v, v2v, 1)
        pct = abs(v1v - v2v) / base
        entry = f"  {key}: V1={v1v} V2={v2v} (diff {pct*100:.0f}%)"
        if pct > tolerance:
            if key in ('title',):
                critical.append(entry)
            else:
                cosmetic.append(entry)
    elif str(v1v) != str(v2v):
        entry = f"  {key}: V1={v1v!r} V2={v2v!r}"
        if key in ('title', 'page_type'):
            critical.append(entry)
        else:
            cosmetic.append(entry)

# Output as parseable report
report = {
    "url": "$URL",
    "v1": v1,
    "v2": v2,
    "critical": critical,
    "cosmetic": cosmetic,
    "status": "FAIL" if critical else ("WARN" if cosmetic else "OK"),
}

if critical:
    print(f"CRITICAL ({len(critical)} fields)")
    for d in critical:
        print(d)
if cosmetic:
    print(f"COSMETIC ({len(cosmetic)} fields)")
    for d in cosmetic:
        print(d)
if not critical and not cosmetic:
    print("PARITY OK")

# Always dump JSON to stderr for machine consumption
print(json.dumps(report), file=sys.stderr)
PYEOF
