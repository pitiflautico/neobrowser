# Releasing

NeoBrowser ships a single static binary that people install with one command and point
their agent at. When a release is broken, the failure is silent: the artefact uploads
fine, `install.sh` hands it out, and nobody finds out until an issue arrives. Everything
below exists to make that impossible.

## What is automated

| Gate | Where | Catches |
|---|---|---|
| fmt, clippy, tests (real Chrome) | `ci.yml`, every PR | regressions before merge |
| `docs/TOOLS.md` matches the registry | `ci.yml`, every PR | public docs drifting from the binary |
| binary runs on all 3 platforms | `release-smoke.yml`, after publish | an artefact that does not start |
| checksums match what was published | `release-smoke.yml` | a corrupt or mismatched upload |
| `install.sh` end to end | `release-smoke.yml` | a broken install path |

The smoke workflow downloads the **published** artefact — the same bytes a user gets —
and makes it prove itself: it reports a version, its tool count agrees with the shipped
docs, and `doctor` launches Chrome and attaches over CDP. Driving a real browser is the
whole product, so that is what gets tested, not a mock.

## Cutting a release

1. Bump the version in `rust/Cargo.toml`.
2. Regenerate the tool docs if tools changed — CI fails otherwise:
   ```sh
   cd rust && cargo run --release -- tools --markdown > ../docs/TOOLS.md
   ```
3. Merge to `main` with CI green.
4. Tag and push:
   ```sh
   git tag v0.1.8 && git push origin v0.1.8
   ```
5. **Wait for `Release smoke` to go green before announcing anything.** Publishing the
   release only proves it compiled.

## If the smoke test fails

Do not delete the tag and re-cut quietly — people may already have the binary. Mark the
release as a pre-release so `install.sh` stops serving it (the installer resolves
`latest`), fix forward, and cut a new patch version.

## Adding a tool

A new tool touches three places, and CI only enforces the last one:

- the registry in `rust/src/tool_impls/`
- `docs/TOOLS.md` — regenerate it, never hand-edit
- the MCP server instructions, if the tool changes the core loop

The tool-count check in the smoke test compares the binary against the count in
`docs/TOOLS.md`, so a forgotten regeneration fails the release rather than shipping
docs that contradict the product.
