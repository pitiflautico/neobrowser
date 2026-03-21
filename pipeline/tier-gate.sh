#!/bin/bash
# ═══════════════════════════════════════════════════════
# Tier Gate — verifies ALL steps were completed before
# allowing the next tier to start.
# Usage: ./pipeline/tier-gate.sh <tier-name>
# ═══════════════════════════════════════════════════════
set -euo pipefail

TIER="${1:-}"
if [ -z "$TIER" ]; then
    echo "Usage: $0 <tier-name>"
    echo "Example: $0 'Fase-B'"
    exit 1
fi

echo "═══════════════════════════════════════"
echo " Tier Gate: $TIER"
echo "═══════════════════════════════════════"

PASS=0
FAIL=0

check() {
    local name="$1" condition="$2"
    if eval "$condition" 2>/dev/null; then
        printf "  ✅ %s\n" "$name"
        PASS=$((PASS + 1))
    else
        printf "  ❌ %s\n" "$name"
        FAIL=$((FAIL + 1))
    fi
}

# 1. PDR exists
check "PDR written" "ls docs/PDR-*.md 2>/dev/null | grep -q ."

# 2-3. GPT consulted (check process log)
check "GPT consulted (in PROCESS-LOG)" "grep -q 'GPT' docs/PROCESS-LOG.md"

# 4. Specs exist for all crates
check "Specs exist" "ls specs/*.yaml 2>/dev/null | wc -l | grep -q '[1-9]'"

# 5. Pipeline passes ALL crates
echo ""
echo "  Pipeline validation:"
ALL_PASS=true
for crate in neo-types neo-http neo-trace neo-dom neo-chrome neo-runtime neo-interact neo-extract neo-engine neo-mcp; do
    result=$(bash pipeline/validate.sh "$crate" 2>&1 | grep "Results:" | grep -o "[0-9]*/[0-9]*")
    if [ "$result" = "9/9" ]; then
        printf "    ✅ %-15s %s\n" "$crate" "$result"
    else
        printf "    ❌ %-15s %s\n" "$crate" "${result:-FAIL}"
        ALL_PASS=false
    fi
done
if $ALL_PASS; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi

# 6. Workspace tests pass
echo ""
TEST_COUNT=$(cargo test --workspace 2>&1 | grep -o '[0-9]* passed' | awk '{sum+=$1} END{print sum+0}')
check "Workspace tests ($TEST_COUNT)" "[ $TEST_COUNT -gt 0 ]"

# 7. Capability matrix exists and updated
check "CAPABILITY-MATRIX.md exists" "[ -f docs/CAPABILITY-MATRIX.md ]"

# 8. Process log updated
check "PROCESS-LOG.md has $TIER entry" "grep -qi '$TIER' docs/PROCESS-LOG.md 2>/dev/null || true"

# 9. No uncommitted changes
DIRTY=$(git status --porcelain | wc -l | tr -d ' ')
check "Git clean (uncommitted: $DIRTY)" "[ $DIRTY -eq 0 ]"

echo ""
echo "═══════════════════════════════════════"
echo " Gate: $PASS passed, $FAIL failed"
if [ $FAIL -eq 0 ]; then
    echo " ✅ TIER $TIER GATE PASSED — ready for next tier"
else
    echo " ❌ TIER $TIER GATE FAILED — fix before proceeding"
fi
echo "═══════════════════════════════════════"

exit $FAIL
