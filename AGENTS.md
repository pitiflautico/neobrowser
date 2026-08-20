# AGENTS.md — orientation for AI agents & contributors

NeoBrowser is an **MCP server that drives a real Google Chrome via the Chrome
DevTools Protocol (CDP)** so AI models can use the web autonomously — stealthily,
and optionally with the user's real logged-in sessions.

There are **two implementations**:

- **`rust/`** — the shipped product. A single ~5.5 MB binary (dynamically linked against the platform libc, not fully static). **All new work goes here.**
- **`archive/python-oracle/`** — the original Python implementation, archived. Not the product, not maintained; its README explains why it stopped being a useful oracle.

## Build, run, test (Rust — primary)

```bash
cd rust
cargo build --release            # -> target/release/neobrowser
cargo test                       # unit + 1 live-Chrome integration test (self-skips w/o Chrome)
cargo fmt --check && cargo clippy --all-targets -- -D warnings   # CI gate
./target/release/neobrowser doctor       # environment check
./target/release/neobrowser tools --markdown > ../docs/TOOLS.md   # regenerate tool docs
python3 scripts/demo.py          # end-to-end demo against real sites
node --version                   # one test syntax-checks the JS embedded in the Rust
```

Archived Python oracle: `cd archive/python-oracle && pip install -e ".[dev]" && python -m pytest -q`.

## How it works (Rust module map)

| module | role |
|---|---|
| `cdp` | CDP protocol client on tokio: one connection task multiplexes commands (routed by id) and events; typed timeouts; drains on disconnect |
| `chrome` | Chrome discovery/launch, stealth flags, health checks; `Drop` reaps the process |
| `browser` | Owns the shared Chrome + tabs; lazy launch/attach; self-healing; multi-tab |
| `capture` | Background listener buffering console + network events per tab |
| `page` | Page-level CDP verbs (navigate, read, click, type, screenshot, find, nudge_frame) |
| `ops` | JS-blob tools (fill, form_fill, submit, extract, analyze, scroll, …) |
| `sessions` | Cookie snapshot/restore, save_session, scripted login |
| `cookies` | Cross-platform real-profile cookie decryption (Keychain/secret-service/DPAPI) |
| `reach` | browse/upload/download with an SSRF guard |
| `search` | Multi-provider search that skips walled sources |
| `policy` | Central pre-dispatch decision: action class, domain allow/deny, profile (`developer`/`safe`/`autonomous`) |
| `action` | Verified-action envelope, time budgets, observe → act → verify |
| `observe` | Accessibility snapshots with stable refs, and diffs between them |
| `vault` | Encrypted-at-rest session material, TTL, verifiable revocation |
| `untrusted` | Fencing and labelling page content; injection detection |
| `trace` | Correlated event timeline, secret redaction, evidence bundles |
| `devtools` | Web Vitals, computed styles, response bodies, HAR export |
| `frames` | Shadow DOM / iframe piercing, dialogs, device emulation |
| `bridge` | Authenticated localhost queue for the Chrome Bridge extension |
| `config` | Versioned config file, env overrides, named profiles |
| `walls` | Generic bot-wall / captcha / consent / rate-limit / login detection |
| `stealth` | JS anti-detection patch (genuine, not spoofed) |
| `llm` | Optional LLM `find` fallback (opt-in via `ANTHROPIC_API_KEY`) |
| `playbook` | Record/replay tool sequences |
| `tools` / `tool_impls` / `mcp` | Tool trait + registry + argument validation + JSON-RPC server |

## Conventions

- **The sandbox is not negotiable**: `--no-sandbox` must never return to `DEFAULT_CHROME_FLAGS`. NeoBrowser drives untrusted pages, so the renderer sandbox is the boundary protecting the host — and the user's live sessions. It comes only from `chrome::resolve_sandbox`, which refuses the unsandboxed + real-profile combination outright. `no_sandbox_is_never_a_default_flag` guards this; if a CI host can't sandbox, fix the host, don't add the flag.
- **Credentials are origin-scoped**: anything secret (cookies, auth headers) stops at the scheme+host+port the caller asked for. Adding a new outbound fetch path means routing it through `reach::guarded_get`, not a bare `reqwest` call.
- **New tools must be classified**: add the tool name to `policy::classify`. Unclassified names fall through to `ActionClass::Script`, the most restrictive class, so forgetting fails closed — and `every_registered_tool_is_classified` will fail until you decide deliberately. The policy check lives in `mcp::handle_tools_call` between validation and dispatch; don't add a second one inside a tool.
- **Never report success you did not observe**: a mutating action returns `ActionStatus::Uncertain` when nothing on the page changed. Do not add a code path that promotes `uncertain` to `succeeded`. If a genuinely successful action reports `uncertain`, the bug is in the state digest's coverage (`action::state_js`) — that is how the shadow-DOM and same-length-text gaps were found — not a reason to loosen the rule.
- **Verify against reality**: new tools get an E2E check against a real page, not just a compile. Prefer `data:` URLs for hermetic tests.
- **Stealth is "real > fake"**: never spoof a value that would mismatch the genuine browser (WebGL, hardwareConcurrency, UA vs Client Hints). See `stealth.rs`.
- **Anti-detection semantics**: clicks are real `isTrusted` mouse events; typing can be per-key with human cadence.
- **Headless renders lazily**: force frames (`page::nudge_frame`) before reading deferred/virtualized content; don't rely on blind waits.
- **Cost discipline**: nothing calls a paid API unless the user opts in (`ANTHROPIC_API_KEY`).
- **Parity is frozen, not maintained**: the Python oracle was useful while the Rust port was catching up. It no longer is. Mutating tools now return the verified-action envelope instead of a confirmation string, so `scripts/compare.py` reports differences by design, and there is no version of that script that could pass without reverting Epic B. Treat `archive/python-oracle/` as a historical reference for the *algorithms* (cookie decryption, wall heuristics) and do not gate changes on it.

## CI and release gates

| Workflow | When | What it gates |
|---|---|---|
| `ci.yml` | every push and PR | fmt, clippy `-D warnings`, the full suite on **macOS, Linux and Windows**, `server.json` version parity, `cargo audit`, `cargo deny`, secret scanning, SBOM |
| `nightly.yml` | 03:00 UTC daily | 12 cells: 3 OSes × Chrome stable/beta × isolated/persistent profile, plus the live bot-detector check. Beta failures are advisory; a stable failure is not |
| `release.yml` | on a `v*` tag | tag/version parity, 5 targets including a **verified-static** musl build, signing when the certificates are configured, provenance attestation, SBOM |

`scripts/verify-release.sh <tag>` is the independent-verification path: it checks
provenance, checksums, the static-musl claim, rebuilds from the tag, runs the rebuilt
binary's own suite, and confirms the README's numbers. It is meant to be run by someone
who does not trust this repository's own claims.

## Using the server (for an AI client)

The `initialize` response includes an `instructions` field summarizing the core loop.
Full per-tool reference: [`docs/TOOLS.md`](docs/TOOLS.md). Introspect live with
`neobrowser tools`.
