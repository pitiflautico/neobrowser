# NeoBrowser

MCP server that gives AI agents a real browser — your session already loaded.  
One Python file. One `websockets` dependency. No Selenium, no Playwright, no chromedriver.

```bash
npx neobrowser
```

Requires Python 3.10+ and Google Chrome.

---

## Quick decision: which tool to use?

```
Need to read a public page?
  → Static/server-rendered (news, docs, wikis)  → browse
  → SPA / JS-heavy / Cloudflare-protected        → open → read

Need to interact with a page?
  → Fill a form, click, type, scroll             → open → find/fill/click/submit
  → Wait for async content to load               → open → wait → read

Need auth-gated content? (Twitter, LinkedIn, GitHub, your apps)
  → open (uses your real Chrome session automatically)

Need to ask ChatGPT / Grok?
  → gpt / grok (dedicated persistent tab, no API key needed)

Need to extract structure?
  → read type=tweets          — Twitter/X feed
  → read type=posts           — blog/Reddit posts
  → read type=comments        — comment threads
  → read type=products        — e-commerce listings
  → read type=table           — HTML tables
  → read type=links           — all href URLs
  → read type=markdown        — full page as markdown
  → read type=accessibility   — semantic a11y tree (most reliable for SPAs)

Need to debug a page?
  → debug (captures console.log, JS errors, uncaught exceptions)

Not sure what's on a page?
  → read (no type = full a11y tree, most informative)
```

---

## Install & Configure

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

Run `npx neobrowser doctor` to verify everything is set up.

---

## Environment variables

NeoBrowser works with zero configuration. These unlock additional features:

| Variable | What it does |
|---|---|
| `NEOBROWSER_PROFILE` | Chrome profile to sync from (default: `Profile 24`). Run `chrome://version` to find yours. |
| `NEOBROWSER_COOKIE_DOMAINS` | Comma-separated domain allowlist for cookie sync, e.g. `github.com,twitter.com`. Default: all non-Google domains. |
| `NEOBROWSER_CONTENT_PROCESS` | Set to `1` to post-process web content through `claude -p` before returning (requires Claude CLI). |
| `OPENAI_API_KEY` | Enables ChatGPT via OpenAI API in the `gpt` tool (more reliable than browser automation). |
| `XAI_API_KEY` | Enables Grok via xAI API in the `grok` tool. |

---

## Tools reference (21 tools)

### browse — fast HTTP fetch, no Chrome

```
browse(url, selector?, prompt?)
```

- Uses HTTP + BeautifulSoup, not Chrome — **fastest path** (0.1–0.8s)
- Falls back to Chrome automatically if HTTP returns <500 chars (SPA detected)
- `selector`: CSS selector to extract a specific part of the page
- `prompt`: LLM filter — extracts only the relevant part via Claude Haiku
- **Use when**: public static pages, docs, news, APIs that return HTML
- **Don't use when**: login required, Cloudflare JS challenge, SPA (React/Vue/Angular)

---

### search — DuckDuckGo web search

```
search(query, num?)
```

- Returns ranked `title + URL` pairs (~1s)
- `num`: number of results (default 10)
- **Use when**: you need to find URLs before reading content
- **Pattern**: `search → browse/open` — search first, then read the best result

---

### open — navigate Chrome to URL

```
open(url, tab?)
```

- Opens URL in Ghost Chrome using your real session cookies
- **Smart SPA wait**: detects when content is actually loaded (not just `readyState=complete`):
  - Twitter/X: waits for `[data-testid=tweetText]` elements
  - LinkedIn: waits for feed cards / scaffold
  - ChatGPT: waits for `#prompt-textarea`
  - GitHub: waits for `main` content area
  - Unknown sites: waits for body text to stabilise (stop growing)
- `tab`: named tab to reuse across calls (e.g. `"docs"` to keep a tab open)
- **Use when**: SPA, login-required, Cloudflare-protected, JS-heavy pages
- **After open**: use `read`, `find`, `click`, `fill`, `submit`, `scroll`, `wait`

---

### read — extract content from current Chrome page

```
read(type?, url?, prompt?)
```

| type | Best for | Cost |
|---|---|---|
| *(none)* | Any page — full semantic a11y tree | expensive |
| `text` | Raw innerText — fastest, no structure | fast |
| `main` | Article/content area, strips nav+footer | fast |
| `headings` | h1–h6 outline for quick page structure | fast |
| `meta` | title + description + og tags | fast |
| `links` | All href links with anchor text | fast |
| `markdown` | Full page converted to markdown | medium |
| `tweets` | Twitter/X feed — tweet text, author, stats | medium |
| `posts` | Blog posts, Reddit threads | medium |
| `comments` | Comment sections, replies | medium |
| `products` | E-commerce product listings with prices | medium |
| `table` | HTML tables as structured text | medium |
| `accessibility` / `a11y` | Full semantic a11y tree (most reliable for complex UIs) | expensive |
| `spatial` / `map` | Elements with bounding-box coordinates (for click-by-position) | expensive |

- `url`: open URL first, then read (shorthand for `open → read`)
- `prompt`: LLM filter — extracts only what you need from the page

---

### find — find interactive elements

```
find(text?, role?, selector?)
```

- Returns element list with indices for use with `click(index=N)`
- `text`: substring match on visible text
- `role`: ARIA role — `button`, `link`, `textbox`, `checkbox`, `combobox`, etc.
- `selector`: CSS selector
- **Use when**: you need to identify which button/link to click before clicking

---

### click — click an element

```
click(text?, selector?, index?)
```

- `text`: clicks first element containing this text
- `selector`: CSS selector
- `index`: index from `find` results (most precise)
- Triggers navigation, toggles, form buttons, tab switches

---

### find_and_click — find then click in one step

```
find_and_click(text?, role?, selector?)
```

- Combines `find` + `click` — useful when you know what you want to click

---

### type — type into a field

```
type(selector, value)
```

- Types into a specific input by CSS selector
- Uses clipboard paste for ProseMirror/contenteditable fields (React-safe)

---

### fill — smart form fill

```
fill(selector, value)
```

- Finds field by CSS selector, sets value, dispatches React-compatible input events
- Works with: `<input>`, `<textarea>`, `<select>`, checkboxes, radio buttons, contenteditable

---

### submit — submit a form

```
submit(selector?)
```

- `selector`: optional submit button CSS selector
- Falls back to pressing Enter if no selector given

---

### scroll — scroll the page

```
scroll(direction, amount?)
```

- `direction`: `"up"` or `"down"`
- `amount`: pixels (default 500)
- **Use when**: lazy-loaded content, infinite scroll feeds, reading long pages

---

### wait — wait for element or text

```
wait(selector?, text?, timeout?)
```

- Polls until selector or text appears (default 5000ms timeout)
- Returns page content when ready
- **Use after**: `open` for pages where content loads asynchronously after navigation

---

### screenshot — capture page

```
screenshot()
```

- Returns base64 PNG of the current Chrome viewport
- **Use when**: verifying page state visually, debugging rendering issues

---

### login — authenticate with credentials

```
login(url, email, password)
```

- Navigates to URL, finds email + password fields, submits
- **Use when**: you have credentials and the session sync didn't handle it
- **Note**: prefer session sync (happens automatically) over manual login when possible

---

### extract — extract structured data

```
extract(type)
```

- `type=links`: all href URLs with anchor text
- `type=tables`: all HTML tables as formatted text

---

### js — execute JavaScript

```
js(code, tab?)
```

- Runs arbitrary JS in the Chrome tab, returns result
- `code` must use `return` statement
- `tab`: target a specific named tab (`"gpt"`, `"grok"`, etc.)
- **Use when**: `read`/`find` can't get what you need, or for page manipulation

---

### debug — capture console logs and JS errors

```
debug(url?, tab?, clear?)
```

- Captures `console.log/warn/error/info` + uncaught JS exceptions + unhandled promise rejections
- `url`: navigate to this URL first with interceptor already active (catches all logs from load)
- `tab`: inspect a specific named tab — `"gpt"` to debug ChatGPT tab, `"grok"` for Grok
- `clear`: set `true` to reset the log buffer after reading
- Returns grouped summary: errors first, then warnings, then logs (last 20 each)
- **Use when**: SPA page not loading correctly, JS errors, diagnosing why a tool call fails
- **Pattern**: `debug(url=X)` → inspect errors → fix → retry

---

### gpt — chat with ChatGPT

```
gpt(message, action?)
```

Actions:
| action | What it does |
|---|---|
| `send` (default) | Send message, wait for full response (returns text) |
| `read_last` | Get the latest response (use if `send` returned `generating` status) |
| `is_streaming` | Check if response is still being generated |
| `history` | Get last N messages from conversation |
| `check_session` | Verify ChatGPT is authenticated |
| `check_input` | Verify input box is ready |

- Uses a dedicated persistent Chrome tab — keeps conversation context across calls
- Does NOT require `OPENAI_API_KEY` — uses your real browser session
- Handles ChatGPT's o1/o3/o4 extended thinking (may take 60–120s)
- **Pattern for long responses**: `gpt(send, message)` → if status=`generating`, poll with `gpt(read_last)` 

---

### grok — chat with Grok

```
grok(message, action?)
```

Same actions as `gpt`. Uses your real X/Twitter session in a dedicated tab.

---

### plugin — run YAML automation pipelines

```
plugin(action, name?, code?)
```

- `action=run name=X`: run a saved plugin
- `action=list`: show all available plugins
- `action=create name=X code=Y`: create a new plugin
- **Use when**: repeatable multi-step browser workflows

---

### status — show system state

```
status()
```

Returns: Chrome PID, open tabs with URLs, connection state, cookie sync stats.

---

## How it works

**Ghost Chrome**: each MCP session launches a headless Chrome with an isolated profile. At exit, the profile is deleted. Between sessions, a persistent `ghost-default` profile caches Chrome's HTTP cache for faster repeat requests.

**Session sync**: at startup, NeoBrowser copies cookies + localStorage + IndexedDB from your real Chrome profile into Ghost Chrome. This means you're already logged into every site you use — no re-authentication needed.

**Anti-detection**: Ghost Chrome uses your exact Chrome version's user-agent string (no "HeadlessChrome" marker). Passes standard bot-detection probes with score=0 (webdriver=undefined, WebGL active, realistic screen size).

**CDP directly**: raw WebSocket to Chrome DevTools Protocol — no Selenium, no Playwright, no chromedriver. One `websockets` dependency.

**Dual fetch path**:
- `browse`: HTTP → BeautifulSoup parse (~0.1–0.8s, no Chrome needed)
- `open`: Chrome CDP → full JS execution, your session, SPA support

---

## Auth-gated sites that work out of the box

These sites work automatically via session sync (no login needed if you're already logged in Chrome):

| Site | What you can do |
|---|---|
| **Twitter / X** | Read feed, tweets, profiles, DMs |
| **LinkedIn** | Read feed, profiles, jobs, messages |
| **GitHub** | Read private repos, issues, PRs, notifications |
| **ChatGPT** | Full chat via `gpt` tool |
| **Grok** | Full chat via `grok` tool |
| **Gmail** | Read emails (via `open` + `read type=accessibility`) |
| **Any site you're logged into** | Works automatically |

**If a site shows a login wall**: call `login` tool or use `gpt(action=check_session)`. NeoBrowser will attempt cookie re-sync automatically.

---

## Session & Security

**What gets copied at startup:**
- Cookies from your Chrome profile (SQLite, read-only, WAL-safe copy)
- localStorage entries
- IndexedDB databases

**What's excluded by default:**
- All Google domains (`.google.com`, `.googleapis.com`, `.youtube.com`, `.gmail.com`) — Google detects duplicate sessions
- Passwords — never copied
- Browsing history — never copied

**Control:**
- Change profile: `NEOBROWSER_PROFILE=Profile 3` (find your profile name at `chrome://version`)
- Sync only specific domains: `NEOBROWSER_COOKIE_DOMAINS=github.com,x.com`
- Add more excluded domains: set `EXCLUDED_DOMAINS` in `neo-browser.py`

---

## Benchmarks

Warm Chrome (after first call):

```
browse (HTTP)          0.11–0.77s
open + SPA wait        0.8–3.0s
read (any type)        0.05–0.2s
find / click / fill    <0.01s
screenshot             0.12s
search (DDG)           0.94s
gpt response           5–60s  (LLM server time, not NeoBrowser)
```

vs. alternatives:
- vs. `fetch`: same speed for static, but handles auth, JS, SPAs
- vs. Playwright: faster startup (no driver), real session, not detectable as bot
- vs. API scraping: works on any site without needing an API

---

## Common patterns

**Read a site you're logged into:**
```
open("https://x.com/home") → read(type="tweets")
open("https://linkedin.com/feed") → read(type="posts")
open("https://github.com/notifications") → read(type="accessibility")
```

**Fill and submit a form:**
```
open(url) → fill(selector, value) × N → submit()
```

**Wait for async content:**
```
open(url) → wait(selector="[data-loaded]") → read()
```

**Debug a broken page:**
```
debug(url=X) → read errors → js(code) to inspect → fix
```

**Multi-turn ChatGPT:**
```
gpt(send, "question 1") → gpt(send, "follow up") → gpt(history)
```

**Search then read:**
```
search("topic") → browse(first_url) or open(first_url)
```

---

## Limitations

- ChatGPT response times vary (5–90s) — LLM server latency, not NeoBrowser
- Cookie sync is one-time at startup — cookies set later in your real Chrome aren't reflected
- Some enterprise WAFs (BotGuard, Akamai) may still block despite real Chrome UA
- macOS + Linux supported; Windows not tested

---

## Tests

```bash
python3 -m pytest tests/ -q           # 176 unit tests, ~0.2s, no Chrome needed
```

---

## Links

- npm: https://www.npmjs.com/package/neobrowser
- GitHub: https://github.com/pitiflautico/neobrowser
- Landing: https://pitiflautico.github.io/neobrowser
