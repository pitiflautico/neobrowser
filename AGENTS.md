# AGENTS.md — orientation for AI agents & contributors

NeoBrowser is an **MCP server that drives a real Google Chrome via the Chrome
DevTools Protocol (CDP)** so AI models can use the web autonomously — stealthily,
and optionally with the user's real logged-in sessions.

There are **two implementations**:

- **`rust/`** — the shipped product. A single static binary. **All new work goes here.**
- **`neobrowser/`** — the original Python implementation, kept as a **differential-testing oracle**. Not the product; don't add features here.

## Build, run, test (Rust — primary)

```bash
cd rust
cargo build --release            # -> target/release/neobrowser
cargo test                       # unit + 1 live-Chrome integration test (self-skips w/o Chrome)
cargo fmt --check && cargo clippy --all-targets -- -D warnings   # CI gate
./target/release/neobrowser doctor       # environment check
./target/release/neobrowser tools --markdown > ../docs/TOOLS.md   # regenerate tool docs
python3 scripts/demo.py          # end-to-end demo against real sites
python3 scripts/compare.py       # Python↔Rust parity gate
```

Python oracle: `pip install -e ".[dev]" && python -m pytest -q`.

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
| `walls` | Generic bot-wall / captcha / consent / rate-limit / login detection |
| `stealth` | JS anti-detection patch (genuine, not spoofed) |
| `llm` | Optional LLM `find` fallback (opt-in via `ANTHROPIC_API_KEY`) |
| `playbook` | Record/replay tool sequences |
| `tools` / `tool_impls` / `mcp` | Tool trait + registry + argument validation + JSON-RPC server |

## Conventions

- **Verify against reality**: new tools get an E2E check against a real page, not just a compile. Prefer `data:` URLs for hermetic tests.
- **Stealth is "real > fake"**: never spoof a value that would mismatch the genuine browser (WebGL, hardwareConcurrency, UA vs Client Hints). See `stealth.rs`.
- **Anti-detection semantics**: clicks are real `isTrusted` mouse events; typing can be per-key with human cadence.
- **Headless renders lazily**: force frames (`page::nudge_frame`) before reading deferred/virtualized content; don't rely on blind waits.
- **Cost discipline**: nothing calls a paid API unless the user opts in (`ANTHROPIC_API_KEY`).
- **Parity**: keep the Rust core behavior-compatible with the Python oracle for shared tools; run `scripts/compare.py`. New capabilities (multi-tab, multi-source search, walls) are Rust-only and expected to differ.

## Using the server (for an AI client)

The `initialize` response includes an `instructions` field summarizing the core loop.
Full per-tool reference: [`docs/TOOLS.md`](docs/TOOLS.md). Introspect live with
`neobrowser tools`.
