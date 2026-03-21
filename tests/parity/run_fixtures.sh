#!/bin/bash
# R2.7 Parity Harness — Run V2 against all fixtures, compare with golden outputs.
# Usage: ./run_fixtures.sh [--update-golden] [tolerance_percent]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
V1_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neobrowser-rs/target/release/neobrowser_rs"
V2_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neorender-v2/target/release/neorender"
FIXTURES_DIR="$SCRIPT_DIR/fixtures"
GOLDEN_DIR="$SCRIPT_DIR/golden"
UPDATE_GOLDEN=0
TOLERANCE=10

# Parse args
for arg in "$@"; do
  case "$arg" in
    --update-golden) UPDATE_GOLDEN=1 ;;
    [0-9]*) TOLERANCE="$arg" ;;
  esac
done

# -- Preflight --
if [ ! -x "$V2_BIN" ]; then
  echo "FATAL: V2 binary not found: $V2_BIN" >&2
  exit 1
fi

# We need a simple HTTP server for file:// fixtures
# Start one temporarily
PORT=18937
cd "$FIXTURES_DIR"
python3 -m http.server $PORT --bind 127.0.0.1 &>/dev/null &
HTTP_PID=$!
trap "kill $HTTP_PID 2>/dev/null || true" EXIT
sleep 0.5

echo "=== Fixture Parity (tolerance ${TOLERANCE}%) ==="
echo ""

total=0
ok=0
warn=0
fail=0

# -- Generate golden files if requested --
if [ "$UPDATE_GOLDEN" -eq 1 ]; then
  if [ ! -x "$V1_BIN" ]; then
    echo "FATAL: V1 binary not found (needed for golden generation): $V1_BIN" >&2
    exit 1
  fi
  echo "Generating golden files from V1..."
  mkdir -p "$GOLDEN_DIR"
  for fixture in "$FIXTURES_DIR"/*.html; do
    name=$(basename "$fixture" .html)
    url="http://127.0.0.1:$PORT/$name.html"
    echo -n "  $name... "
    golden_out=$(echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"'"$url"'","mode":"neorender"}}}' \
      | timeout 30 "$V1_BIN" mcp 2>/dev/null \
      | python3 -c "
import json, sys
for line in sys.stdin:
    try:
        d = json.loads(line.strip())
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
                    'scripts': data.get('scripts', 0),
                }, indent=2))
                break
    except: pass
" 2>/dev/null || echo '{}')
    echo "$golden_out" > "$GOLDEN_DIR/$name.json"
    echo "done"
  done
  echo ""
fi

# -- Run V2 against each fixture, compare with golden --
for fixture in "$FIXTURES_DIR"/*.html; do
  name=$(basename "$fixture" .html)
  url="http://127.0.0.1:$PORT/$name.html"
  golden_file="$GOLDEN_DIR/$name.json"
  total=$((total + 1))

  echo -n "[$name] "

  if [ ! -f "$golden_file" ]; then
    echo "SKIP (no golden file — run with --update-golden first)"
    continue
  fi

  # Run V2
  v2_out=$(timeout 30 "$V2_BIN" see "$url" 2>/dev/null | python3 -c "
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
        'wom_nodes': len(nodes),
    }))
except:
    print('{}')
" 2>/dev/null || echo '{}')

  # Compare
  result=$(python3 << PYEOF
import json
golden = json.load(open("$golden_file"))
v2 = json.loads('''$v2_out''') if '''$v2_out'''.strip() else {}
tolerance = $TOLERANCE / 100.0

critical = []
cosmetic = []

for key in ['title', 'links', 'forms', 'buttons', 'inputs', 'page_type']:
    gv = golden.get(key, '')
    v2v = v2.get(key, '')
    if isinstance(gv, int) and isinstance(v2v, int):
        if gv == 0 and v2v == 0:
            continue
        base = max(gv, v2v, 1)
        pct = abs(gv - v2v) / base
        if pct > tolerance:
            entry = f"{key}: golden={gv} v2={v2v} ({pct*100:.0f}%)"
            if key == 'title':
                critical.append(entry)
            else:
                cosmetic.append(entry)
    elif str(gv) != str(v2v):
        entry = f"{key}: golden={gv!r} v2={v2v!r}"
        if key in ('title', 'page_type'):
            critical.append(entry)
        else:
            cosmetic.append(entry)

if critical:
    print("CRITICAL: " + "; ".join(critical))
elif cosmetic:
    print("WARN: " + "; ".join(cosmetic))
else:
    print("OK")
PYEOF
)

  echo "$result"

  case "$result" in
    OK*) ok=$((ok + 1)) ;;
    WARN*) warn=$((warn + 1)) ;;
    CRITICAL*) fail=$((fail + 1)) ;;
  esac
done

echo ""
echo "=== Summary: $total fixtures — $ok ok, $warn warn, $fail critical ==="
