#!/usr/bin/env bash
# Compare V1 neobrowser MCP against V2 neo-mcp using compact browse-style tools.
set -euo pipefail

V1_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neobrowser-rs/target/release/neobrowser_rs"
V2_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neorender-v2/target/release/neorender"
TIMEOUT=30
OUTDIR="/tmp/neorender-mcp-bench"

mkdir -p "$OUTDIR"

SITES=(
  "hackernews|https://news.ycombinator.com"
  "wikipedia|https://en.wikipedia.org/wiki/Rust_(programming_language)"
  "github|https://github.com/nicbarker/clay"
  "reddit|https://old.reddit.com"
)

if [ ! -x "$V1_BIN" ]; then
  echo "FATAL: V1 binary not found: $V1_BIN" >&2
  exit 1
fi

if [ ! -x "$V2_BIN" ]; then
  echo "FATAL: V2 binary not found: $V2_BIN" >&2
  exit 1
fi

extract_metrics() {
  local file=$1
  python3 - "$file" <<'PY'
import json, re, sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(errors="replace")
payload = ""

for line in text.splitlines():
    line = line.strip()
    if not line or not line.startswith("{"):
        continue
    try:
        data = json.loads(line)
    except Exception:
        continue
    if "result" in data:
        content = data["result"].get("content", [])
        if content:
            payload = content[0].get("text", "")

links = payload.count("→")
print(f"{len(payload.encode())}\t{links}")
PY
}

run_v1() {
  local url=$1 out=$2 err=$3
  local start end elapsed
  start=$(python3 -c "import time; print(time.time())")
  printf '%s\n' \
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"browser_open\",\"arguments\":{\"url\":\"$url\",\"mode\":\"neorender\"}}}" \
    | timeout "$TIMEOUT" "$V1_BIN" mcp >"$out" 2>"$err" || true
  end=$(python3 -c "import time; print(time.time())")
  elapsed=$(python3 -c "start=float('$start'); end=float('$end'); print(f'{end - start:.2f}')")
  echo "$elapsed"
}

run_v2() {
  local url=$1 out=$2 err=$3
  local start end elapsed
  start=$(python3 -c "import time; print(time.time())")
  printf '%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"browse\",\"arguments\":{\"url\":\"$url\"}}}" \
    | timeout "$TIMEOUT" "$V2_BIN" mcp >"$out" 2>"$err" || true
  end=$(python3 -c "import time; print(time.time())")
  elapsed=$(python3 -c "start=float('$start'); end=float('$end'); print(f'{end - start:.2f}')")
  echo "$elapsed"
}

printf "%-12s | %8s | %8s | %10s | %10s | %8s | %8s\n" \
  "Site" "V1 MCP" "V2 MCP" "V1 Bytes" "V2 Bytes" "V1 Lnk" "V2 Lnk"
printf "%-12s-|-%8s-|-%8s-|-%10s-|-%10s-|-%8s-|-%8s\n" \
  "------------" "--------" "--------" "----------" "----------" "--------" "--------"

for entry in "${SITES[@]}"; do
  IFS='|' read -r label url <<< "$entry"

  v1_out="$OUTDIR/${label}_v1.jsonl"
  v1_err="$OUTDIR/${label}_v1.err"
  v2_out="$OUTDIR/${label}_v2.jsonl"
  v2_err="$OUTDIR/${label}_v2.err"

  v1_time=$(run_v1 "$url" "$v1_out" "$v1_err")
  v2_time=$(run_v2 "$url" "$v2_out" "$v2_err")

  IFS=$'\t' read -r v1_bytes v1_links <<< "$(extract_metrics "$v1_out")"
  IFS=$'\t' read -r v2_bytes v2_links <<< "$(extract_metrics "$v2_out")"

  printf "%-12s | %8ss | %8ss | %10s | %10s | %8s | %8s\n" \
    "$label" "$v1_time" "$v2_time" "$v1_bytes" "$v2_bytes" "$v1_links" "$v2_links"
done

echo ""
echo "Raw MCP outputs saved in: $OUTDIR"
