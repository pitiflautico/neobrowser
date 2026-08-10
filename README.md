# NeoBrowser

**Your AI drives a real Chrome with your real logged-in sessions — no login walls, no CAPTCHAs, and it doesn't get flagged as a bot.** An MCP server for AI models to use the web the way you do.

Most browser tools for LLMs launch a fresh, fingerprintable headless browser with no cookies, so the model hits login walls and bot checks constantly. NeoBrowser drives the **real Google Chrome binary** and can reuse **your actual logged-in profile**, so the model lands already authenticated and looks like a genuine user — because it *is* one.

```jsonc
// Add to your MCP client (Claude Code, Claude Desktop, Cursor, …)
{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
```

> Rust rewrite: a single ~4 MB static binary, no runtime to install. (The original Python implementation lives on in this repo as a test oracle — see [Development](#development).)

---

## Install

```bash
# One line (macOS / Linux):
curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh

# Or from source (needs the Rust toolchain):
git clone https://github.com/pitiflautico/neobrowser && cd neobrowser/rust
cargo build --release        # -> target/release/neobrowser

neobrowser doctor            # verify Chrome is found + a live CDP smoke test
```

Windows binaries are on the [Releases](https://github.com/pitiflautico/neobrowser/releases) page. Requires Google Chrome (or Chromium); auto-discovered on macOS/Linux/Windows, override with `NEOBROWSER_CHROME_BIN`.

## See it work

```bash
python3 rust/scripts/demo.py     # drives a real login, file upload, and a bot-detector check
```

Real output against live sites:

```
✓ Open a real login page             Navigated to .../login
✓ Fill the username / password       ok
✓ Click Login (real isTrusted click) ok
✓ Read the result → logged in        You logged into a secure area!
✓ Attach a real image file           ok
✓ Submit the upload                  ok
✓ Server confirms the file           neobrowser_demo.png
✓ Check the stealth tells            {"webdriver":"hidden (passed)","chrome_runtime":true,"headless_ua":false}
```

## Why NeoBrowser

| | NeoBrowser | Playwright MCP / Puppeteer | browser-use |
|---|:---:|:---:|:---:|
| Drives the **real Chrome binary** | ✅ | ⚠️ bundled Chromium | ⚠️ |
| Reuses your **real logged-in sessions** (no API keys, no re-login) | ✅ | ❌ | ❌ |
| Stealth by default — passes bot.sannysoft with a **genuine** fingerprint | ✅ | ❌ | partial |
| **Semantic** element finding (accessibility tree + heuristics + optional LLM) | ✅ | ❌ selectors | ✅ |
| **Multi-source** search that routes around bot walls | ✅ | ❌ | ❌ |
| Single static binary, zero runtime deps | ✅ | ❌ Node + browsers | ❌ |
| Talks CDP directly (no Selenium/WebDriver) | ✅ | — | — |

## Features

- **Real-session browsing** — optionally decrypt + inject cookies from your real Chrome profile (opt-in; macOS Keychain / Linux secret-service / Windows DPAPI). Session-identity cookies for Google/LinkedIn/Microsoft are excluded so your real browser isn't logged out.
- **Stealth-hardened, genuinely** — real Chrome, `navigator.webdriver` suppressed, real-version User-Agent matching its Client Hints, **real GPU WebGL** (not spoofed). The philosophy is consistency, not piling on fakes. Verified live against bot.sannysoft.
- **Bot-wall aware** — `navigate` detects bot walls, CAPTCHAs, consent gates, rate-limits and login gates on any site and tells the model how to react.
- **Multi-source search** — text (DuckDuckGo + Google), images (Bing + Google), videos (YouTube + Google): walled sources are skipped, results merged. No single site is a hard dependency.
- **Real multi-tab** — `new_tab` / `list_tabs` / `switch_tab` / `close_tab`, all sharing one Chrome.
- **43 tools** — navigate, click, type, fill/submit forms, upload/download, read, extract tables, screenshot, scroll, console/network logs, performance metrics, record/replay playbooks, web/image/video search, login, and more.
- **Robust core** — one isolated CDP connection per tab (tokio), typed timeouts, self-healing recovery from dead tabs / restarted Chrome, and no orphaned Chrome processes.

## Documentation

- **[docs/TOOLS.md](docs/TOOLS.md)** — full reference for all 43 tools (params + descriptions). Regenerate with `neobrowser tools --markdown`; introspect live with `neobrowser tools`.
- **[AGENTS.md](AGENTS.md)** — architecture, build/test, and conventions for contributors and AI agents.
- The MCP `initialize` response ships an `instructions` field so the model gets a usage primer automatically.

## Usage

Register it with any MCP client, then ask your model to browse. Example tool calls:

```
navigate   { "url": "https://example.com" }
find       { "intent": "search box" }        → returns a backendNodeId
type       { "text": "hello world" }
screenshot { "format": "png" }                → returned as an image
read       {}                                 → visible page text
```

By default NeoBrowser runs its own headless Chrome under a dedicated profile. To reuse your real logged-in sessions, set `NEOBROWSER_REAL_PROFILE` (see below).

## Real-session mode

Set `NEOBROWSER_REAL_PROFILE` to the Chrome profile folder whose sessions you want (e.g. `"Default"`, `"Profile 1"`). NeoBrowser decrypts that profile's cookies via the OS keychain and injects them, so the agent starts authenticated:

```jsonc
{ "mcpServers": { "neobrowser": {
  "command": "neobrowser",
  "env": { "NEOBROWSER_REAL_PROFILE": "Default" }
} } }
```

Or attach to a Chrome you already have open (started with `--remote-debugging-port=9222`): set `NEOBROWSER_ATTACH_PORT=9222`. In attach mode NeoBrowser never patches or kills your real browser.

## Stealth

Modern bot detection (Cloudflare, DataDome, …) mostly looks for **inconsistencies** — a spoofed UA that doesn't match Client Hints, a `HeadlessChrome` token, software WebGL, `navigator.webdriver === true`. NeoBrowser is **genuinely consistent** rather than piling on spoofs:

- Runs the **real Chrome binary** (real TLS, real fonts, real everything).
- `navigator.webdriver` forced `undefined`; anti-throttle + focus emulation keep the headless compositor live so content actually renders.
- UA rewritten to the **real installed Chrome version** via the launch flag, so genuine Client Hints stay consistent.
- No `--disable-gpu`, so WebGL reports the **real GPU**.
- JS patches for `plugins`, `languages`, and the permissions/`Notification` mismatch — only on tabs NeoBrowser owns, never on an attached real Chrome.

Verified live: passes bot.sannysoft's WebDriver, Chrome, plugins and WebGL checks with the host's genuine fingerprint.

## Configuration

| Env var | Default | Purpose |
|---|---|---|
| `NEOBROWSER_REAL_PROFILE` | *(unset)* | Real Chrome profile folder to pull sessions from |
| `NEOBROWSER_ATTACH_PORT` | *(unset)* | Attach to an already-running Chrome on this debug port |
| `NEOBROWSER_CHROME_BIN` | *(auto)* | Path to the Chrome/Chromium binary |
| `NEOBROWSER_HOME` | `~/.neobrowser` | Where profiles, cookies, sessions, playbooks, downloads live |
| `NEOBROWSER_PROXY` | *(unset)* | Upstream proxy (`http://…` or `socks5://…`) |
| `NEOBROWSER_DISABLE_GPU` | *(unset)* | Force software rendering (GPU-less CI hosts only) |
| `ANTHROPIC_API_KEY` | *(unset)* | Enables the optional LLM fallback in `find` (your key, your cost; off by default) |

## Security & responsible use

Real-session mode reads cookies from your Chrome profile and injects them into an automated browser. Treat it like any credential:

- It is **opt-in** — nothing touches your real profile unless you set `NEOBROWSER_REAL_PROFILE`.
- Cookie/session files are written under `~/.neobrowser` with `0600` permissions.
- Server-side fetches (`browse`, `download`) are **SSRF-guarded** to public http(s) only.
- The `login` tool refuses non-`https` URLs and never logs credentials.
- Anything an AI browses with your session acts **as you**. Point it only at sites and tasks you'd be comfortable doing yourself. This is a tool for automating *your own* accounts and workflows — not for evading access controls on services you don't own.

## Development

```bash
# Rust (primary):
cd rust && cargo test          # unit + one live-Chrome integration test (self-skips without Chrome)
cargo test --test stealth_verify -- --ignored   # real bot.sannysoft detector

# Python (legacy implementation, kept as a differential-testing oracle):
pip install -e ".[dev]" && python -m pytest -q
```

## License

MIT © Daniel Perez Pinazo
