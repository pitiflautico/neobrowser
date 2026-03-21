#!/usr/bin/env bash
# ── NeoRender Benchmark: V1 (neobrowser-rs) vs V2 (neorender-v2) ──
# Compares light-mode (no Chrome) on 8 real sites.
# V1 command: fetch <url>  (text output)
# V2 command: see <url>    (JSON output)

set -euo pipefail

V1_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neobrowser-rs/target/release/neobrowser_rs"
V2_BIN="/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neorender-v2/target/release/neorender"
TIMEOUT=60
OUTDIR="/tmp/neorender-bench"
REPORT="/tmp/neorender-benchmark-v1v2.txt"

mkdir -p "$OUTDIR"

# ── Sites ──
SITES=(
  "wikipedia|https://en.wikipedia.org/wiki/Rust_(programming_language)"
  "github|https://github.com/nicbarker/clay"
  "hackernews|https://news.ycombinator.com"
  "mdn|https://developer.mozilla.org/en-US/docs/Web/JavaScript"
  "stackoverflow|https://stackoverflow.com/questions/tagged/rust"
  "reddit|https://old.reddit.com"
  "nytimes|https://www.nytimes.com"
  "youtube|https://www.youtube.com"
)

# ── Helpers ──
count_links_text() {
  # Count link-like patterns in V1 text output (lines starting with [link] or containing href)
  grep -ciE '^\[link\]|→|https?://' "$1" 2>/dev/null || echo 0
}

count_links_json() {
  # Count links in V2 JSON output
  python3 -c "
import json,sys
try:
    d=json.load(open('$1'))
    nodes=d.get('wom',{}).get('nodes',[])
    print(sum(1 for n in nodes if n.get('role')=='link'))
except: print(0)
" 2>/dev/null
}

human_size() {
  local bytes=$1
  if [ "$bytes" -ge 1048576 ]; then
    echo "$(echo "scale=1; $bytes / 1048576" | bc)M"
  elif [ "$bytes" -ge 1024 ]; then
    echo "$(echo "scale=1; $bytes / 1024" | bc)K"
  else
    echo "${bytes}B"
  fi
}

# ── Check binaries ──
V1_OK=true
V2_OK=true

if [ ! -x "$V1_BIN" ]; then
  echo "WARNING: V1 binary not found at $V1_BIN"
  V1_OK=false
fi
if [ ! -x "$V2_BIN" ]; then
  echo "ERROR: V2 binary not found at $V2_BIN"
  V2_OK=false
  exit 1
fi

echo "═══════════════════════════════════════════════════════════════"
echo "  NeoRender Benchmark — V1 vs V2 (light mode, no Chrome)"
echo "  $(date '+%Y-%m-%d %H:%M:%S')"
echo "  V1: $V1_BIN"
echo "  V2: $V2_BIN"
echo "  Timeout: ${TIMEOUT}s per site"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# ── Results arrays ──
declare -a R_SITE R_V1_TIME R_V2_TIME R_V1_CONTENT R_V2_CONTENT R_V1_LINKS R_V2_LINKS R_V1_EXIT R_V2_EXIT

run_one() {
  local label=$1 url=$2 engine=$3 bin=$4 cmd=$5 outfile=$6
  local t_start t_end elapsed exit_code

  echo -n "  [$engine] $label ... "
  t_start=$(python3 -c "import time; print(time.time())")

  if timeout "$TIMEOUT" "$bin" $cmd "$url" > "$outfile" 2>"${outfile}.err"; then
    exit_code=0
  else
    exit_code=$?
  fi

  t_end=$(python3 -c "import time; print(time.time())")
  elapsed=$(python3 -c "print(f'{$t_end - $t_start:.2f}')")

  local content_len
  content_len=$(wc -c < "$outfile" | tr -d ' ')

  echo "${elapsed}s | $(human_size "$content_len") | exit=$exit_code"

  # Return via globals (bash limitation)
  _TIME="$elapsed"
  _CONTENT="$content_len"
  _EXIT="$exit_code"
}

# ── Main loop ──
idx=0
for entry in "${SITES[@]}"; do
  IFS='|' read -r label url <<< "$entry"
  echo "── $label ($url)"

  # V1
  if $V1_OK; then
    run_one "$label" "$url" "V1" "$V1_BIN" "fetch" "$OUTDIR/v1_${label}.out"
    R_V1_TIME[$idx]="$_TIME"
    R_V1_CONTENT[$idx]="$_CONTENT"
    R_V1_EXIT[$idx]="$_EXIT"
    R_V1_LINKS[$idx]=$(count_links_text "$OUTDIR/v1_${label}.out")
  else
    R_V1_TIME[$idx]="N/A"
    R_V1_CONTENT[$idx]="0"
    R_V1_EXIT[$idx]="N/A"
    R_V1_LINKS[$idx]="0"
  fi

  # V2
  run_one "$label" "$url" "V2" "$V2_BIN" "see" "$OUTDIR/v2_${label}.out"
  R_V2_TIME[$idx]="$_TIME"
  R_V2_CONTENT[$idx]="$_CONTENT"
  R_V2_EXIT[$idx]="$_EXIT"
  R_V2_LINKS[$idx]=$(count_links_json "$OUTDIR/v2_${label}.out")

  R_SITE[$idx]="$label"
  idx=$((idx + 1))
  echo ""
done

# ── Report ──
echo ""
echo "═══════════════════════════════════════════════════════════════════════════════════════════════"
echo "  RESULTS TABLE"
echo "═══════════════════════════════════════════════════════════════════════════════════════════════"
printf "%-14s | %8s | %8s | %6s | %10s | %10s | %8s | %6s | %6s\n" \
  "Site" "V1 Time" "V2 Time" "Ratio" "V1 Content" "V2 Content" "Parity%" "V1 Lnk" "V2 Lnk"
printf "%-14s-|-%8s-|-%8s-|-%6s-|-%10s-|-%10s-|-%8s-|-%6s-|-%6s\n" \
  "--------------" "--------" "--------" "------" "----------" "----------" "--------" "------" "------"

GATE_SPEED_FAIL=0
GATE_CONTENT_FAIL=0
TOTAL_SITES=$idx

for ((i=0; i<idx; i++)); do
  site="${R_SITE[$i]}"
  v1t="${R_V1_TIME[$i]}"
  v2t="${R_V2_TIME[$i]}"
  v1c="${R_V1_CONTENT[$i]}"
  v2c="${R_V2_CONTENT[$i]}"
  v1l="${R_V1_LINKS[$i]}"
  v2l="${R_V2_LINKS[$i]}"

  # Compute ratio and parity
  if [[ "$v1t" == "N/A" ]]; then
    ratio="N/A"
    parity="N/A"
  else
    ratio=$(python3 -c "
v1=$v1t; v2=$v2t
print(f'{v2/v1:.1f}x' if v1 > 0 else 'INF')
")
    parity=$(python3 -c "
v1=$v1c; v2=$v2c
print(f'{v2/v1*100:.0f}%' if v1 > 0 else 'INF')
")
    # Check gates
    speed_ratio=$(python3 -c "v1=$v1t; v2=$v2t; print(v2/v1 if v1>0 else 999)")
    content_ratio=$(python3 -c "v1=$v1c; v2=$v2c; print(v2/v1 if v1>0 else 0)")

    if python3 -c "exit(0 if $speed_ratio > 2.0 else 1)"; then
      GATE_SPEED_FAIL=$((GATE_SPEED_FAIL + 1))
    fi
    if python3 -c "exit(0 if $content_ratio < 0.9 else 1)"; then
      GATE_CONTENT_FAIL=$((GATE_CONTENT_FAIL + 1))
    fi
  fi

  printf "%-14s | %8s | %8s | %6s | %10s | %10s | %8s | %6s | %6s\n" \
    "$site" "${v1t}s" "${v2t}s" "$ratio" "$(human_size "$v1c")" "$(human_size "$v2c")" "$parity" "$v1l" "$v2l"
done

echo ""
echo "═══════════════════════════════════════════════════════════════════════════════════════════════"
echo "  REGRESSION GATES"
echo "═══════════════════════════════════════════════════════════════════════════════════════════════"

if ! $V1_OK; then
  echo "  [SKIP] V1 binary not available — gates cannot be evaluated"
  echo "  Only V2 results are reported above."
else
  if [ "$GATE_SPEED_FAIL" -eq 0 ]; then
    echo "  [PASS] Speed: V2 is NOT >2x slower than V1 on any site"
  else
    echo "  [FAIL] Speed: V2 is >2x slower than V1 on $GATE_SPEED_FAIL/$TOTAL_SITES sites"
  fi

  if [ "$GATE_CONTENT_FAIL" -eq 0 ]; then
    echo "  [PASS] Content: V2 extracts >= 90% of V1 content on all sites"
  else
    echo "  [FAIL] Content: V2 extracts < 90% of V1 content on $GATE_CONTENT_FAIL/$TOTAL_SITES sites"
  fi

  if [ "$GATE_SPEED_FAIL" -eq 0 ] && [ "$GATE_CONTENT_FAIL" -eq 0 ]; then
    echo ""
    echo "  ✅ ALL GATES PASSED"
  else
    echo ""
    echo "  ❌ SOME GATES FAILED"
  fi
fi

echo ""
echo "Raw outputs saved in: $OUTDIR/"
echo "Report: $REPORT"
