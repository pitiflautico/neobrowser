# NeoBrowser

**A fast, stealthy MCP browser-automation server that drives real Chrome with your real logged-in sessions — built for AI models to navigate the web autonomously.**

Most browser tools for LLMs launch a fresh, fingerprintable headless browser with no cookies, so the model hits login walls and bot checks constantly. NeoBrowser takes the opposite approach: it drives the **real Google Chrome binary** and can reuse **your actual logged-in profile**, so the model lands already authenticated and looks like a genuine user — because it *is* one.

```jsonc
// Add to your MCP client (Claude Code, Claude Desktop, etc.)
{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
```

---

## Why NeoBrowser

| | NeoBrowser | Playwright MCP / Puppeteer | browser-use |
|---|:---:|:---:|:---:|
| Drives the **real Chrome binary** | ✅ | ⚠️ bundled Chromium | ⚠️ |
| Reuses your **real logged-in sessions** (no API keys, no re-login) | ✅ | ❌ | ❌ |
| Stealth by default (no `navigator.webdriver`, genuine UA + Client Hints, real WebGL) | ✅ | ❌ | partial |
| **Semantic** element finding (accessibility tree + heuristics + optional LLM) | ✅ | ❌ selectors | ✅ |
| Runtime dependencies | `websockets` (+ optional `anthropic`) | Node + browsers | many |
| Talks CDP directly (no Selenium/WebDriver) | ✅ | — | — |

The result: an agent that opens `linkedin.com/messaging`, `x.com`, or your dashboard and is **already logged in**, moving at the speed of the network instead of getting stuck on auth walls and CAPTCHAs.

## Features

- **Real-session browsing** — optionally sync cookies + storage from your real Chrome profile (opt-in; see [Security](#security--privacy)).
- **Stealth-hardened** — genuine Chrome, `navigator.webdriver` suppressed, real-version User-Agent that matches its Client Hints, real GPU WebGL, JS-level fingerprint patches. See [Stealth](#stealth).
- **Semantic navigation** — `find("send button")` resolves via the CDP accessibility tree + heuristics, with an optional Claude Haiku fallback, instead of brittle CSS selectors.
- **~35 tools** — navigate, click, type, fill/submit forms, read, extract tables, screenshot, scroll, console/network logs, performance metrics, record/replay playbooks, web search, and more.
- **Robust core** — one isolated CDP WebSocket per tab, a thread-safe tab pool with health-checking, and self-healing recovery from dead tabs / restarted Chrome.
- **Zero heavy deps** — pure Python standard library plus `websockets`; `anthropic` only if you want the LLM find fallback.

## Install

```bash
pip install neobrowser        # from PyPI
# or from source:
git clone https://github.com/danielperezpinazo/neobrowser && cd neobrowser
pip install -e .

neobrowser doctor             # check Python, websockets, and Chrome
```

Requires Python 3.10+ and Google Chrome (or Chromium) installed. Chrome is auto-discovered on macOS/Linux/Windows; override with `NEOBROWSER_CHROME_BIN`.

## Usage

Register it with any MCP client:

```jsonc
{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
```

Then ask your model to browse. Example tool calls the model can make:

```
navigate   { "url": "https://example.com" }
find       { "intent": "search box" }        → returns a backendNodeId
type       { "text": "hello world" }
screenshot { "format": "png" }                → returned as an image
read       {}                                 → visible page text
```

By default NeoBrowser runs its own headless Chrome under a dedicated profile. To reuse your real logged-in sessions, see below.

## Real-session mode

Set `NEOBROWSER_REAL_PROFILE` to the name of the Chrome profile folder whose sessions you want (e.g. `"Default"`, `"Profile 1"`). NeoBrowser decrypts that profile's cookies via the OS keychain and injects them, so the agent starts authenticated:

```jsonc
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser",
      "env": { "NEOBROWSER_REAL_PROFILE": "Default" }
    }
  }
}
```

Session-identity cookies for Google, LinkedIn, and Microsoft are deliberately **not** file-synced, because those services log your real browser out when they detect a duplicate session. Everything else (preferences, and post-launch cookie injection) is fair game.

## Stealth

Modern bot detection (Cloudflare, DataDome, …) mostly looks for **inconsistencies** — a spoofed User-Agent that doesn't match the browser's Client Hints, a `HeadlessChrome` token, software WebGL, `navigator.webdriver === true`. NeoBrowser's philosophy is to be **genuinely consistent** rather than to pile on spoofs:

- Runs the **real Chrome binary** (real TLS, real fonts, real everything).
- `--disable-blink-features=AutomationControlled` + a JS patch so `navigator.webdriver` is `undefined`, in headless too.
- The User-Agent is rewritten to the **real installed Chrome version** (no `HeadlessChrome`), and because it's applied via the launch flag rather than a CDP override, the browser's genuine Client Hints stay perfectly consistent with it.
- No `--disable-gpu`, so WebGL reports the **real GPU** (ANGLE/Metal) instead of SwiftShader.
- JS-level patches for `plugins`, `languages`, and the permissions/`Notification` mismatch, injected only into tabs NeoBrowser owns — never into your real attached Chrome.

Verified live: `navigator.webdriver` hidden, UA reports the true Chrome version with matching `Sec-CH-UA`, and WebGL shows the real renderer.

## Configuration

| Env var | Default | Purpose |
|---|---|---|
| `NEOBROWSER_REAL_PROFILE` | *(unset)* | Real Chrome profile folder to pull sessions from |
| `NEOBROWSER_PROFILE` | `default` | Name of the dedicated ghost profile |
| `NEOBROWSER_HOME` | `~/.neobrowser` | Where profiles, cookies, sessions, playbooks live |
| `NEOBROWSER_CHROME_BIN` | *(auto)* | Path to the Chrome/Chromium binary |
| `NEOBROWSER_POOL_SIZE` | `3` | Tab pool size |
| `NEOBROWSER_ATTACH_PORT` | *(unset)* | Attach to an already-running Chrome on this debug port |
| `NEOBROWSER_DISABLE_GPU` | *(unset)* | Force software rendering (GPU-less CI hosts only) |
| `NEOBROWSER_LOG_LEVEL` | `INFO` | Log verbosity |

## Architecture

```
chrome_process   launch/health-check Chrome (stealth flags, cross-platform discovery)
      ↓
session          one Chrome per named profile, anti-zombie health checks
      ↓
tab_pool         thread-safe pool of reusable tabs, health-checked before reuse
      ↓
chrome_tab       one isolated CDP WebSocket per tab, background reader thread
      ↓
page_analyzer    semantic find: accessibility tree + heuristics + optional Haiku
      ↓
browser          one high-level facade over all of the above
```

`cookie_sync` handles session persistence; `playbook` records and replays action sequences; `server` exposes everything as MCP tools over JSON-RPC on stdin/stdout.

## Security & privacy

Real-session mode reads cookies from your Chrome profile and injects them into an automated browser. Treat that with the same care as any credential:

- It is **opt-in** — nothing touches your real profile unless you set `NEOBROWSER_REAL_PROFILE`.
- Cookie and session files are written under `~/.neobrowser` with `0600` permissions.
- The `login` tool refuses non-`https` URLs and never logs credentials, but driving logins from an LLM is inherently sensitive — point it only at destinations you trust.
- Anything an AI model browses with your session acts **as you**. Run it against sites and tasks you'd be comfortable performing yourself.

## Development

```bash
pip install -e ".[dev]"
python3 -m pytest tests/ -q
```

## License

MIT © Daniel Perez Pinazo
