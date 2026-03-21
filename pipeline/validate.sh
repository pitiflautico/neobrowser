#!/bin/bash
# ═══════════════════════════════════════════════════════
# NeoRender V2 — Pipeline Validator
# Runs ALL steps for a crate. Returns 0 only if ALL pass.
# Usage: ./pipeline/validate.sh neo-http
# ═══════════════════════════════════════════════════════
set -euo pipefail

CRATE="${1:-}"
if [ -z "$CRATE" ]; then
    echo "Usage: $0 <crate-name>"
    echo "Example: $0 neo-http"
    exit 1
fi

CRATE_DIR="crates/$CRATE"
if [ ! -d "$CRATE_DIR" ]; then
    echo "ERROR: $CRATE_DIR not found"
    exit 1
fi

PASS=0
FAIL=0
TOTAL=0
RESULTS=""

step() {
    local num="$1" name="$2"
    TOTAL=$((TOTAL + 1))
    echo ""
    echo "── Step $num: $name ──"
}

pass() {
    PASS=$((PASS + 1))
    RESULTS="${RESULTS}  ✅ Step $1: $2\n"
    echo "  ✅ PASS"
}

fail() {
    FAIL=$((FAIL + 1))
    RESULTS="${RESULTS}  ❌ Step $1: $2 — $3\n"
    echo "  ❌ FAIL: $3"
}

echo "═══════════════════════════════════════"
echo " Pipeline: $CRATE"
echo " $(date '+%Y-%m-%d %H:%M:%S')"
echo "═══════════════════════════════════════"

# ── Step 1: Spec exists ──
step 1 "Spec exists"
if [ -f "specs/$CRATE.yaml" ]; then
    pass 1 "Spec exists"
else
    fail 1 "Spec exists" "specs/$CRATE.yaml not found"
fi

# ── Step 2: Scaffold compiles ──
step 2 "Scaffold compiles"
if cargo check -p "$CRATE" 2>/dev/null; then
    pass 2 "Scaffold compiles"
else
    fail 2 "Scaffold compiles" "cargo check failed"
fi

# ── Step 3: No todo!() in non-test code ──
step 3 "Implementation complete"
TODO_COUNT=$(grep -r 'todo!()' "$CRATE_DIR/src/" 2>/dev/null | grep -v '#\[cfg(test)\]' | grep -v 'mod tests' | wc -l | tr -d ' ')
if [ "$TODO_COUNT" = "0" ]; then
    pass 3 "No todo!()"
else
    fail 3 "Implementation" "$TODO_COUNT todo!() found in src/"
fi

# ── Step 4: Tests pass ──
step 4 "Tests pass"
TEST_OUTPUT=$(cargo test -p "$CRATE" 2>&1)
if echo "$TEST_OUTPUT" | grep -q "test result: ok"; then
    TEST_COUNT=$(echo "$TEST_OUTPUT" | grep "test result" | grep -o '[0-9]* passed' | head -1)
    pass 4 "Tests ($TEST_COUNT)"
else
    fail 4 "Tests" "cargo test failed"
fi

# ── Step 5: Clippy zero warnings ──
step 5 "Clippy clean"
if cargo clippy -p "$CRATE" -- -D warnings 2>/dev/null; then
    pass 5 "Clippy"
else
    fail 5 "Clippy" "warnings found"
fi

# ── Step 6: Format clean ──
step 6 "Format clean"
if cargo fmt -p "$CRATE" -- --check 2>/dev/null; then
    pass 6 "Format"
else
    fail 6 "Format" "needs cargo fmt"
fi

# ── Step 7: Doc comments ──
step 7 "Documentation"
DOC_WARNINGS=$(cargo doc -p "$CRATE" --no-deps 2>&1 | grep "missing documentation" | wc -l | tr -d ' ')
if [ "$DOC_WARNINGS" = "0" ]; then
    pass 7 "Docs"
else
    fail 7 "Docs" "$DOC_WARNINGS missing doc comments"
fi

# ── Step 8: File size limits ──
step 8 "Size limits"
SIZE_OK=true
for f in $(find "$CRATE_DIR/src" -name "*.rs" 2>/dev/null); do
    lines=$(wc -l < "$f")
    if [ "$lines" -gt 300 ]; then
        echo "  $f: $lines lines (max 300)"
        SIZE_OK=false
    fi
done
if $SIZE_OK; then
    pass 8 "Size limits"
else
    fail 8 "Size limits" "files exceed 300 lines"
fi

# ── Step 9: No unwrap outside tests ──
step 9 "No unwrap()"
# Count unwrap() only in non-test code: for each .rs file, take lines before #[cfg(test)]
UNWRAP_COUNT=0
for f in $(find "$CRATE_DIR/src" -name "*.rs" 2>/dev/null); do
    test_line=$(grep -n '#\[cfg(test)\]' "$f" 2>/dev/null | head -1 | cut -d: -f1)
    if [ -n "$test_line" ]; then
        count=$(head -n "$((test_line - 1))" "$f" | grep '\.unwrap()' | grep -v '// safe:' | wc -l | tr -d ' ')
    else
        count=$(grep '\.unwrap()' "$f" | grep -v '// safe:' | wc -l | tr -d ' ')
    fi
    UNWRAP_COUNT=$((UNWRAP_COUNT + count))
done
if [ "$UNWRAP_COUNT" = "0" ]; then
    pass 9 "No unwrap()"
else
    fail 9 "No unwrap()" "$UNWRAP_COUNT unwrap() calls found"
fi

# ── Summary ──
echo ""
echo "═══════════════════════════════════════"
echo " Results: $PASS/$TOTAL passed, $FAIL failed"
echo "═══════════════════════════════════════"
echo -e "$RESULTS"

# ── Write metrics ──
mkdir -p pipeline/results
cat > "pipeline/results/$CRATE.json" << JSON
{
  "crate": "$CRATE",
  "timestamp": "$(date -u '+%Y-%m-%dT%H:%M:%SZ')",
  "total": $TOTAL,
  "passed": $PASS,
  "failed": $FAIL,
  "result": "$([ $FAIL -eq 0 ] && echo 'PASS' || echo 'FAIL')"
}
JSON

exit $FAIL
