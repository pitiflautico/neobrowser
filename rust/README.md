# NeoBrowser (Rust)

A fast, stealthy MCP browser-automation server that drives real Chrome via the
Chrome DevTools Protocol — the Rust port of the Python `neobrowser`, built for
robustness and single-binary distribution.

## Why the Rust port

- **Single static binary** (~3.8 MB, no Python runtime for end users).
- **Robust concurrency core**: one tokio connection task per tab multiplexes CDP
  commands and events with `select!` — responses routed by id (no cross-talk),
  typed timeouts, and pending requests drained on disconnect (no 30s hangs).
- **Self-healing sessions** and `Drop`-based Chrome cleanup (no orphan processes).
- **Cross-platform cookie decryption** (macOS Keychain / Linux secret-service /
  Windows DPAPI) — the Python only did macOS.
- **Verified stealth**: a live-Chrome test asserts every headless tell is clean,
  and an on-demand test passes the real bot.sannysoft detector.

## Build & run

```bash
cargo build --release          # -> target/release/neobrowser
./target/release/neobrowser doctor   # check Chrome discovery + a live CDP smoke test
```

Register with any MCP client:

```jsonc
{ "mcpServers": { "neobrowser": { "command": "/path/to/target/release/neobrowser" } } }
```

Env: `NEOBROWSER_CHROME_BIN`, `NEOBROWSER_HOME` (default `~/.neobrowser`),
`NEOBROWSER_PROXY`, `NEOBROWSER_DISABLE_GPU`, `NEOBROWSER_REAL_PROFILE`.

## Tools (43 — full Python parity + Rust additions)

39 Python-parity tools plus 4 Rust-native multi-tab tools:

`status navigate read screenshot find click type js page_info analyze fill
form_fill submit find_and_click dismiss_overlay extract extract_table scroll wait
paginate console_logs network_log metrics debug save_cookies restore_cookies
save_session session_info login browse upload download search search_images
search_videos search_twitter_videos record_task stop_recording replay`
· **multi-tab:** `new_tab list_tabs switch_tab close_tab`

### Beyond parity (Rust-only robustness)

- **Real-session auto-auth** — `NEOBROWSER_REAL_PROFILE` reads + decrypts the real
  Chrome profile's cookies and injects them at launch. Identity and fingerprint
  cookies for Google/Gmail, Microsoft, LinkedIn and other high-risk providers are
  excluded by default to avoid logging your real browser out. If a provider still
  revokes the session, use `NEOBROWSER_ATTACH_PORT` or a logged-in agent profile.
- **Multi-provider search** — text (DuckDuckGo + Google), images (Bing + Google),
  videos (YouTube + Google): walled sources are detected and skipped, results merged.
- **Generic wall detection** — `navigate` flags bot walls, captchas, consent gates,
  rate-limits and login gates on any site.
- **Attach mode** — `NEOBROWSER_ATTACH_PORT` drives a Chrome you already have open
  (no launch, no stealth patching, never killed on exit).

## Tests

```bash
cargo test                              # 61 unit + 1 live-Chrome integration
cargo test --test stealth_verify -- --ignored   # real bot.sannysoft detector
```

## Layout

`cdp` (protocol) · `chrome` (process mgr) · `browser` (session) · `capture`
(console/network events) · `page` (CDP verbs) · `ops` (JS-blob tools) · `sessions`
(cookies/login) · `reach` (browse/upload/download) · `search` · `playbook` ·
`stealth` · `cookies` (cross-platform decrypt) · `tools`/`tool_impls`/`mcp`.

## Cutover from the Python server

The Python package remains in `../neobrowser/` as the differential-testing oracle.
To switch a client to the Rust server, point its MCP `command` at the release
binary above. Retiring the Python package is a deliberate, separate step (update
`pyproject.toml`'s entry point / remove the package) — do it only after validating
the Rust server against your real workflows.
