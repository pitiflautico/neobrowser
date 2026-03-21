#!/bin/bash
# Generate a task spec for an agent
# Usage: ./pipeline/gen-spec.sh neo-http "0.2 Unified HTTP Client"
set -euo pipefail

CRATE="${1:-}"
PHASE="${2:-}"

if [ -z "$CRATE" ] || [ -z "$PHASE" ]; then
    echo "Usage: $0 <crate-name> <phase-description>"
    echo "Example: $0 neo-http '0.2 Unified HTTP Client'"
    exit 1
fi

mkdir -p specs

cat > "specs/$CRATE.yaml" << EOF
# ═══════════════════════════════════════
# Task Spec: $CRATE
# Phase: $PHASE
# Generated: $(date '+%Y-%m-%d %H:%M')
# ═══════════════════════════════════════

crate: $CRATE
phase: "$PHASE"

# Paste the trait interface this crate must implement
trait_interface: |
  // TODO: define trait

# Crates this one depends on (from workspace)
dependencies:
  - neo-types

# Files the agent MUST create
files_to_create:
  - crates/$CRATE/Cargo.toml
  - crates/$CRATE/src/lib.rs
  - crates/$CRATE/src/mock.rs

# Files the agent CANNOT touch
files_forbidden:
  - "crates/neo-engine/*"
  - "crates/neo-mcp/*"
  - "src/main.rs"
  - "Cargo.toml"  # workspace root

# What makes this crate DONE
acceptance_criteria:
  - "trait defined and exported in lib.rs"
  - "real implementation exists"
  - "mock implementation exists"
  - "3+ unit tests pass"
  - "pipeline/validate.sh $CRATE exits 0"
EOF

echo "✅ Created specs/$CRATE.yaml"
echo "   Edit trait_interface and acceptance_criteria before launching agent."
