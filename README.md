# NeoBrowser

MCP server that gives AI agents a real browser.
One Python file. One dependency. Your session already loaded.

---

## Install

```bash
npx neobrowser
```

Requires Python 3.10+ and Chrome.

Run `npx neobrowser doctor` to check everything is set up.

---

## Configure

**Claude Code** (`~/.claude/mcp.json`):
```json
{
  "neo-browser": {
    "command": "npx",
    "args": ["-y", "neobrowser"]
  }
}
```

**Claude Desktop** (`~/Library/Application Support/Claude/claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "neo-browser": {
      "command": "npx",
      "args": ["-y", "neobrowser"]
    }
  }
}
```

---

## What it does

**Example 1: Search and read**
```
You: search for "rust async runtime" and open the first result
→ neo-browser calls search("rust async runtime") → 5 results in 0.9s
→ then open(first_url) → page content in 1.2s
```

**Example 2: Fill a form**
```
You: go to httpbin.org/forms/post and fill the form with test data
→ neo-browser calls open(url) → page loaded in 0.9s
→ then fill(5 fields) → filled in <0.01s
→ then submit() → form submitted
```

**Example 3: Ask ChatGPT**
```
You: ask ChatGPT what it thinks about MCP servers
→ neo-browser opens dedicated ChatGPT tab (first time: ~10s)
→ types message, sends, waits for response
→ returns ChatGPT's answer via conversation API
```

---

## Tools (19)

### HTTP — no Chrome required

| Tool | Description |
|---|---|
| `browse` | Fast HTTP fetch + smart parse (~0.1–0.8s) |
| `search` | DuckDuckGo web search (~1s) |

### Chrome browsing

| Tool | Description |
|---|---|
| `open` | Navigate to URL in Ghost Chrome |
| `read` | Extract content: markdown, a11y tree, tweets, posts, tables, products |
| `find` | Find element by text, CSS, XPath, or ARIA role |
| `click` | Click by text or CSS selector |
| `type` | Type in input — finds field by label, placeholder, name, or aria-label |
| `fill` | Smart fill: inputs, textareas, selects, checkboxes, radios |
| `submit` | Submit form |
| `scroll` | Scroll page |
| `wait` | Wait for element or text to appear |
| `login` | Automated email + password login |
| `extract` | Extract tables or links |
| `screenshot` | Capture PNG |
| `js` | Execute arbitrary JavaScript |

### AI Chat — dedicated persistent tabs

| Tool | Description |
|---|---|
| `gpt` | ChatGPT: `send`, `read_last`, `is_streaming`, `history` |
| `grok` | Grok: `send`, `read_last`, `is_streaming`, `history` |

### Meta

| Tool | Description |
|---|---|
| `plugin` | Run, list, or create YAML automation pipelines |
| `status` | Chrome state, open tabs, PIDs |

---

## How it works

- **Ghost Chrome**: headless Chrome per MCP process, isolated profile, deleted on exit
- **Session sync**: cookies + localStorage + IndexedDB copied from your real Chrome at startup
- **Dual path**: `browse` uses HTTP for static pages, `open` uses Chrome CDP for SPAs and auth-gated pages
- No Selenium, no Playwright, no chromedriver — raw CDP over WebSocket, one `websockets` dependency

---

## Session & Security

**What gets copied at startup:**
- Cookies from your Chrome profile (SQLite, read-only, WAL-safe)
- localStorage entries
- IndexedDB databases
- SessionStorage

**What's excluded by default:**
- Google domains: `.google.com`, `.googleapis.com`, `.youtube.com`, `.gmail.com`
- Reason: Google detects duplicate sessions and logs out your real browser

**Which profile:**
- Default: the profile set in the `PROFILE` constant in `neo-browser.py` (currently `Profile 24`)
- Logged on startup: `[neo] Session sync from Profile 24: 5332 cookies kept, 398 Google excluded`

**What's NOT shared:**
- Passwords — never copied
- Autofill data — never copied
- Browsing history — never copied
- The ghost profile is deleted on exit

**Control:**
- To change profile: set `PROFILE` constant in `neo-browser.py`
- To exclude more domains: add to `EXCLUDED_DOMAINS` tuple
- To disable session sync: remove the real Chrome profile path

---

## Benchmarks

All times wall-clock, macOS, measured with `benchmark.py`.

```
Cold start:      1.6s    (Chrome launch + cookie sync, one-time per session)
19 operations:   7.1s    (0.37s avg)
ChatGPT:        33.0s    (mostly LLM server response time)
```

Per operation, warm:

```
browse (HTTP)        0.11–0.77s
open (Chrome)        0.48–1.32s
read                 0.10s avg
fill / find / click  <0.01s
screenshot           0.12s
search (DDG)         0.94s
```

---

## Limitations

- ChatGPT response times vary (5–60s+) — server dependent, not NeoBrowser overhead
- Cookie sync is one-time at startup — cookies set later in your real Chrome are not reflected
- CDP `insertText` doesn't work in dedicated chat tabs (uses DOM fallback instead)
- macOS tested, Linux should work, Windows not tested
- Some enterprise WAFs may still block despite real Chrome UA

---

## CLI

```
neo-browser.py              Start MCP server
neo-browser.py --help       Show help
neo-browser.py --version    Show version
neo-browser.py doctor       Check dependencies
```

---

## License

MIT

---

## Links

- npm: https://www.npmjs.com/package/neobrowser
- GitHub: https://github.com/pitiflautico/neobrowser
- Landing: https://pitiflautico.github.io/neobrowser
