# Orchestration Protocol

## How Claude orchestrates V2 development

### 1. Wave Execution

I execute one wave at a time. Each wave can have parallel agents.

```
WAVE 1: neo-types + neo-http + neo-trace (parallel)
WAVE 2: neo-dom + neo-chrome (parallel, depends on Wave 1)
WAVE 3: neo-runtime (depends on Wave 2)
WAVE 4: neo-interact + neo-extract (parallel, depends on Wave 2)
WAVE 5: neo-engine (depends on Wave 3 + 4)
WAVE 6: neo-mcp (depends on Wave 5)
```

### 2. Per-Crate Agent Launch Sequence

For each crate in a wave, I:

1. **Generate spec**: Create `specs/{crate}.yaml` with trait interface
2. **Launch agent** with isolation (worktree or separate context)
3. **Agent prompt includes**:
   - The spec YAML
   - The engineering standards
   - The pipeline validation requirement
   - Files it can/cannot modify
4. **Agent executes**: scaffold → implement → test → pipeline
5. **Agent returns**: summary + `pipeline/validate.sh` output
6. **I verify**: check pipeline output, review changes
7. **Merge**: if pipeline passes, merge to main

### 3. Agent Prompt Template

```
You are building crate `{CRATE}` for NeoRender V2.

## Task Spec
{CONTENTS OF specs/{CRATE}.yaml}

## Location
All your work goes in: crates/{CRATE}/
You CANNOT modify any file outside this directory.

## Workflow (follow in order, do NOT skip steps)

### Step 1: Read
Read the trait interface in the spec. Understand what you're building.

### Step 2: Cargo.toml
Create crates/{CRATE}/Cargo.toml with:
- name = "{CRATE}"
- workspace dependencies
- Only the dependencies listed in the spec

### Step 3: Scaffold
Create src/lib.rs with:
- Module declarations
- The trait from the spec (copy exactly)
- Re-exports

Create src/{module}.rs for each module with:
- Struct definition
- `impl Trait for Struct` with todo!() bodies
- Doc comments on every pub item

Create src/mock.rs with:
- Mock struct
- `impl Trait for MockStruct` with todo!() bodies

### Step 4: Verify scaffold
Run: cargo check -p {CRATE}
Must compile (todo!() is fine at this stage).

### Step 5: Implement
Fill in all todo!() with real logic.
Follow these rules:
- Max 300 lines per file
- Max 50 lines per function
- thiserror for errors, NEVER anyhow
- NEVER unwrap() outside tests
- Doc comment on every pub fn/struct/trait

### Step 6: Implement mock
Fill in MockStruct with:
- Configurable responses
- Request recording
- Useful for tests

### Step 7: Write tests
Create tests/{module}_test.rs with:
- At least 3 unit tests using the mock
- Test the happy path
- Test an error case
- Test an edge case

Also add #[cfg(test)] mod tests {} in each src file.

### Step 8: Run pipeline
Run: bash pipeline/validate.sh {CRATE}
If ANY step fails, fix the issue and re-run.
Keep fixing until ALL steps pass.

### Step 9: Report
Return:
1. Summary of what you built
2. Full output of pipeline/validate.sh
3. List of files created/modified
```

### 4. Integration Check Between Waves

After each wave completes:

```bash
# Full workspace check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

If integration fails, I identify which crate broke and send it back to its agent.

### 5. State File

Track progress in `pipeline/state.json`:

```json
{
  "current_wave": 1,
  "crates": {
    "neo-types": {"wave": 0, "status": "done", "pipeline": "pass"},
    "neo-http": {"wave": 1, "status": "in_progress", "agent": "a123"},
    "neo-trace": {"wave": 1, "status": "in_progress", "agent": "b456"},
    "neo-dom": {"wave": 2, "status": "pending"},
    "neo-chrome": {"wave": 2, "status": "pending"},
    "neo-runtime": {"wave": 3, "status": "pending"},
    "neo-interact": {"wave": 4, "status": "pending"},
    "neo-extract": {"wave": 4, "status": "pending"},
    "neo-engine": {"wave": 5, "status": "pending"},
    "neo-mcp": {"wave": 6, "status": "pending"}
  }
}
```

### 6. Recovery Protocol

If an agent fails:
1. Read its output to understand what went wrong
2. Create a follow-up spec with the error context
3. Launch a new agent with the fix instructions
4. OR fix it myself if it's small

If a wave is blocked:
1. Identify the blocking crate
2. Escalate: try a different approach
3. If still blocked: skip the feature, document as limitation
