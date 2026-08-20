# NeoBrowser

[![CI](https://github.com/pitiflautico/neobrowser/actions/workflows/ci.yml/badge.svg)](https://github.com/pitiflautico/neobrowser/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/pitiflautico/neobrowser?sort=semver)](https://github.com/pitiflautico/neobrowser/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Install in VS Code](https://img.shields.io/badge/VS_Code-Install-0098FF?style=flat-square&logo=visualstudiocode&logoColor=white)](https://vscode.dev/redirect/mcp/install?name=neobrowser&config=%7B%22command%22%3A%22neobrowser%22%7D)
[![Install in Cursor](https://img.shields.io/badge/Cursor-Install-000000?style=flat-square&logo=cursor&logoColor=white)](https://cursor.com/en/install-mcp?name=neobrowser&config=eyJjb21tYW5kIjoibmVvYnJvd3NlciJ9)
[![GitHub stars](https://img.shields.io/github/stars/pitiflautico/neobrowser?style=flat-square)](https://github.com/pitiflautico/neobrowser/stargazers)
[![Landing](https://img.shields.io/badge/Website-pitiflautico.github.io/neobrowser-5eead4?style=flat-square)](https://pitiflautico.github.io/neobrowser/)

**An MCP server that drives a real Google Chrome with your real logged-in sessions — and that reports what actually happened, not what it attempted.**

```jsonc
// Add to your MCP client (Claude Code, Claude Desktop, Cursor, …)
{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
```

Or just paste this into Claude Code / Cursor / Codex and let the agent wire it up:

```text
Install neobrowser (brew install neobrowser or curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh), register it as an MCP server named neobrowser, and use it to browse the real Chrome profile I already have logged in. If Chrome is not installed, report that instead of guessing.
```

## A status the caller can act on

Ask any browser-automation tool to click a button and it will usually tell you it succeeded. What it means is that it dispatched two mouse events at some coordinates. Whether the click landed, whether the page changed, whether the button was there at all — none of that is in the answer.

A human driving a browser never notices the gap, because a human looks at the screen. An agent cannot. It takes the success at face value and continues into a page it never changed, so every step after that reasons from a state that does not exist — and the final report says the task was completed. An error stops an agent; a false success makes it keep going.

So every mutating action here returns a status derived from an observation taken *before* the action, an observation taken *after*, and a detected difference between them — plus the evidence behind it. A click that dispatched but changed nothing reports `uncertain`, never success. `uncertain` is not a failure: it is the honest answer when the tool did its part and could not see the result, and it is one a caller can do something with — retry, escalate, ask a human. There is no recovery from a confident wrong answer.

`ok` is not a field that code can set. It is derived from the status, so an envelope claiming `ok: true` alongside `status: "uncertain"` is unrepresentable rather than merely discouraged (`uncertain_never_serializes_as_ok` holds it that way).

### The Verified Action Contract

That rule is written down as a specification rather than left as an implementation detail: **[docs/VERIFIED-ACTIONS.md](docs/VERIFIED-ACTIONS.md)** — version 1.0, 6 statuses, 10 normative invariants, 13 conformance scenarios, published under CC0. It is implementation-neutral: NeoBrowser is one implementation, and the contract says nothing about how a page must be observed, only about what the report must mean.

The invariants are the things that are easy to lose under pressure. `uncertain` is never promoted to `succeeded` — not on retry, not by a default. `succeeded` requires two observations and a difference between them. A page that cannot be observed yields an *empty* observation, never a cached earlier one, because returning the last known state makes an action that did nothing look like one that worked. `blocked` must name the obstruction. A human gate is reported, never defeated. And the status never depends on which mechanism was used, so a fallback that happens to be easier to verify cannot earn a stronger status than the primary path.

Each invariant maps to a conformance scenario, and several scenarios are defined by what must *not* come back: clicking a button with no handler must report `uncertain` and must not report `succeeded`; clicking under an overlay must report `blocked` and say what covered it.

### How the claim is checked

The scenarios are executable, so the claim is a test result rather than a paragraph in a README:

```bash
cd rust && cargo test --test conformance
```

The suite drives a real Chrome, because that is the only place the failures it looks for actually appear — a shadow root the state digest could not see into, a text change of identical length, a page that never settles, a browser killed mid-action. It **self-skips when no Chrome is present**, the same way the other live tests do, so a partial setup still gives a useful run. That has a consequence worth stating plainly: **a skip is not a pass.** A conformance claim requires the run to have executed, and per §6.2 of the contract a partial pass is reported as a partial pass — there is no "mostly conformant".

The contract is CC0 so that a tool competing with this one can adopt it without asking. A contract only becomes a standard if other people can implement it.

## Real Chrome, real sessions

**A report is only as useful as the page it describes, and an agent that spends its run on a login wall never gets to the task.** So the other half of this tool is getting in: it wins the fingerprint game (passes bot.sannysoft with a genuine fingerprint), moves the mouse like a human, and lands already authenticated, so it isn't flagged like a stock headless bot.

It doesn't pretend to be invisible: when a site throws an interactive challenge (reCAPTCHA, Turnstile) NeoBrowser **detects** it and hands control back with a real-session or human path — that honesty is what makes it dependable.

Most browser tools for LLMs launch a fresh, fingerprintable headless browser with no cookies, so the model hits login walls and bot checks constantly. NeoBrowser drives the **real Google Chrome binary** and can reuse **your actual logged-in profile**, so the model lands already authenticated and looks like a genuine user — because it *is* one.

One deliberate exception, so the promise stays accurate: **Google, LinkedIn and Microsoft session-identity cookies are excluded** from the import. Copying those would log you out of your own browser, and the single-session enforcement on those providers makes a clone a liability rather than a shortcut. Everything else comes across; for those three, expect to log in inside the agent profile once. `session_info` reports a per-provider coverage state (`authenticated` / `partially_authenticated` / `no_session`) and names which excluded providers appear, so an agent can log in once instead of looping on an auth wall. `profile_mode` states which of the three session modes is active and what it means for your credentials.

> Rust rewrite: a single ~6.3 MB binary — no Node, no Python, no browser download. The default Linux build links glibc; a **genuinely static** musl build (`neobrowser-x86_64-unknown-linux-musl`) is published per release for Alpine and for older hosts, and CI fails the release if that artifact turns out not to be static. (The original Python implementation is archived under [`archive/python-oracle/`](archive/python-oracle/).)

---

## Install

```bash
# One line (macOS / Linux). Resolves a specific release, verifies its SHA-256, and
# verifies build provenance when the GitHub CLI is available:
curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh

# Pin a version instead of taking the latest:
NEOBROWSER_VERSION=v0.1.7 curl -fsSL .../install.sh | sh

# Homebrew (macOS / Linux):
brew tap pitiflautico/neobrowser https://github.com/pitiflautico/neobrowser
brew install neobrowser

# Scoop (Windows):
scoop install https://raw.githubusercontent.com/pitiflautico/neobrowser/main/packaging/neobrowser.scoop.json

# Or from source (needs the Rust toolchain):
git clone https://github.com/pitiflautico/neobrowser && cd neobrowser/rust
cargo build --release        # -> target/release/neobrowser

neobrowser doctor            # verify Chrome, sandbox, policy, vault + a live CDP test
neobrowser doctor --json     # same checks, machine-readable; exits non-zero on failure
```

Windows binaries are on the [Releases](https://github.com/pitiflautico/neobrowser/releases) page. Requires Google Chrome (or Chromium); auto-discovered on macOS/Linux/Windows, override with `NEOBROWSER_CHROME_BIN`.

## See it work

![NeoBrowser demo](docs/assets/demo.gif)

*Real run: login, file upload and a bot-detector check against live sites (~14 s).*

### Real sessions vs. a fresh headless browser

![Headless browser blocked at login wall vs NeoBrowser using real Chrome and reaching the dashboard](docs/assets/neobrowser-vs-headless.gif)

*Same target, same prompt: a sterile headless browser hits the login wall; NeoBrowser lands already authenticated because it drives your real Chrome profile.*

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
| Imports sessions from your **existing Chrome profile** [^sessions] | ✅ | ❌ | ❌ |
| Stealth by default — passes bot.sannysoft with a **genuine** fingerprint | ✅ | ❌ | partial |
| **Semantic** element finding (accessibility tree + heuristics + optional LLM) | ✅ | ❌ selectors | ✅ |
| **Multi-source** search that routes around bot walls | ✅ | ❌ | ❌ |
| Single binary, no language runtime to install | ✅ | ❌ Node + browsers | ❌ |
| Talks CDP directly (no Selenium/WebDriver) | ✅ | — | — |

[^sessions]: Specifically: decrypting cookies out of the Chrome profile you already
use, so an agent starts authenticated without you logging in again. This is not the
same as "no persistence elsewhere" — Playwright MCP does keep sessions across runs
via a persistent profile (`--user-data-dir`) or `storageState`, and can attach to
your own Chrome through its extension. What it does not do is adopt the profile you
were already logged into.

## Features

- **Real-session browsing** — optionally decrypt + inject cookies from your real Chrome profile (opt-in; macOS Keychain / Linux secret-service / Windows DPAPI). Session-identity cookies for Google/LinkedIn/Microsoft are excluded so your real browser isn't logged out.
- **Stealth-hardened, genuinely** — real Chrome, `navigator.webdriver` suppressed, real-version User-Agent matching its Client Hints, **real GPU WebGL** (not spoofed). The philosophy is consistency, not piling on fakes. Verified live against bot.sannysoft.
- **Bot-wall aware** — `navigate` detects bot walls, CAPTCHAs, consent gates, rate-limits and login gates on any site and tells the model how to react.
- **Multi-source search** — text (DuckDuckGo + Google), images (Bing + Google), videos (YouTube + Google): walled sources are skipped, results merged. No single site is a hard dependency.
- **Real multi-tab** — `new_tab` / `list_tabs` / `switch_tab` / `close_tab`, all sharing one Chrome.
- **Verified actions** — every mutating action returns a typed envelope with `status` (`succeeded` / `failed` / `blocked` / `needs_human` / `requires_confirmation` / `uncertain`) and the evidence behind it. A click that dispatched but changed nothing reports `uncertain`, never success. The rule is specified in [The Verified Action Contract](docs/VERIFIED-ACTIONS.md) and checked by the conformance suite.
- **Stable element references** — `observe` returns refs like `button:Continue#0` that are re-resolved against the live tree on every use, so they survive the re-render that invalidates a `backendNodeId`. `observe(diff=true)` returns only what changed.
- **Central policy engine** — every call is classified and evaluated before it runs: domain allow/deny lists plus `developer` / `safe` / `autonomous` profiles. Refusals are structured, with a `remedy`.
- **Encrypted session vault** — cookies and localStorage sealed with a key from the OS credential store, with a TTL and verifiable revocation.
- **67 tools** (26 advertised by default) — navigate, observe, click/press/hover/drag, fill/submit forms, set checkboxes and selects, upload/download, read, extract tables and paginated lists, screenshot, console/network logs, Web Vitals, HAR export, computed styles, record/replay playbooks, multi-source search, login. `NEOBROWSER_TOOLSET=full` advertises them all.
- **Reaches what selectors cannot** — `pierce` walks open shadow roots and same-origin iframes; `list_frames` names the cross-origin ones so a missing element is explainable rather than a mystery. `dialog` answers a blocking `alert`/`confirm` that would otherwise look like a hung browser.
- **Chrome Bridge** — an optional [extension](extension/) that lets an agent drive tabs you explicitly share, one at a time, revocably. Your real session with no clone, and no `--remote-debugging-port` exposing every tab. See [extension/README.md](extension/README.md).
- **Robust core** — one isolated CDP connection per tab (tokio), typed timeouts, self-healing recovery from dead tabs / restarted Chrome, and no orphaned Chrome processes.

## Documentation

- **[docs/VERIFIED-ACTIONS.md](docs/VERIFIED-ACTIONS.md)** — The Verified Action Contract: the statuses, the invariants behind them, and the conformance scenarios. CC0, versioned independently of this implementation.
- **[docs/TOOLS.md](docs/TOOLS.md)** — full reference for all 67 tools (params + descriptions). Regenerate with `neobrowser tools --markdown`; introspect live with `neobrowser tools`.
- **[AGENTS.md](AGENTS.md)** — architecture, build/test, and conventions for contributors and AI agents.
- **[extension/README.md](extension/README.md)** — the Chrome Bridge and its security model.
- **[docs/REPRODUCIBILITY.md](docs/REPRODUCIBILITY.md)** — what release provenance guarantees, and where byte-identical rebuilds do not hold yet.
- **[SECURITY.md](SECURITY.md)** — the threat model, what is explicitly *not* defended against, and the scope for an external audit.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — the one rule that matters, and the conventions the tests enforce.
- The MCP `initialize` response ships an `instructions` field so the model gets a usage primer automatically.

## Benchmark

A reproducible harness ([`bench/`](bench/)) drives browser tools through a shared task
matrix, including a 2-way comparison vs Playwright MCP (`python3 bench/compare.py`).
Current per-task numbers live in [`bench/compare.md`](bench/compare.md), regenerated by
the script — deliberately not copied here, because a single run on one machine is not
evidence for a marketing claim.

Two rules the harness holds itself to, after an earlier version broke both:

- **Every tool gets its native capabilities.** Playwright MCP is given its persistent
  profile (`--user-data-dir`) and driven through its own file-chooser flow. An earlier
  revision withheld both and reported the resulting failures as capability gaps —
  they were harness bugs. Notably, the claim that Playwright MCP cannot persist
  sessions was simply **wrong**.
- **Tasks measure outcomes, not tool names.** "Does a cookie survive a browser
  restart" rather than "does a `save_cookies` tool exist", so either design can win on
  merit. The shutdown is also identical for both — SIGTERM to the browser process
  only, since a blanket kill takes out the child that owns the cookie store and makes
  any profile look non-persistent.

Metrics separate `task_execution_success` from `destination_access_success` so a
detected wall never inflates a score. Adversarial pages are **observational only**:
single IP, single run, no "evades better" claim — that needs residential proxies and
repeated runs. What this harness is *not* yet: repeated runs with confidence
intervals, a third-party MCP server in the comparison, or an independent reproduction.
Until it is, treat it as a regression check, not a league table.

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

Beyond the fingerprint, input is **behaviorally human**: clicks move the cursor to the target along a multi-step path with human-cadence pauses (not a teleport-then-click), and typing can be per-key with realistic timing — the signals behavioral systems watch for.

Verified live: passes bot.sannysoft's WebDriver, Chrome, plugins and WebGL checks with the host's genuine fingerprint. **CI installs Chrome and runs these checks against a real browser on every push**; the full bot.sannysoft run is an on-demand test (`cargo test --test stealth_verify -- --ignored`).

What no tool can promise is defeating *interactive challenges* — reCAPTCHA, Turnstile, or behavioral/reputation systems (DataDome) can still put up a wall, and a fresh cookie-less profile is itself a signal. NeoBrowser's edge there is a warm real profile plus **detecting** the wall (`navigate` flags it) so the model reacts instead of hammering it.

## Configuration

| Env var | Default | Purpose |
|---|---|---|
| `NEOBROWSER_REAL_PROFILE` | *(unset)* | Real Chrome profile folder to pull sessions from |
| `NEOBROWSER_PROFILE` | `default` | Which Ghost profile this session uses. Chrome locks a profile exclusively, so give concurrent sessions different names to keep them from colliding |
| `NEOBROWSER_ATTACH_PORT` | *(unset)* | Attach to an already-running Chrome on this debug port |
| `NEOBROWSER_ATTACH_TIMEOUT` | `5` | Seconds to wait when attaching to an existing Chrome (max `120`) |
| `NEOBROWSER_LAUNCH_TIMEOUT` | `15` | Seconds to wait for a fresh Chrome launch (max `120`) |
| `NEOBROWSER_SEND_TIMEOUT` | `30` | Seconds to wait for a CDP command response (max `120`) |
| `NEOBROWSER_CHROME_BIN` | *(auto)* | Path to the Chrome/Chromium binary |
| `NEOBROWSER_HOME` | `~/.neobrowser` | Where profiles, cookies, sessions, playbooks, downloads live |
| `NEOBROWSER_PROXY` | *(unset)* | Upstream proxy (`http://…` or `socks5://…`) |
| `NEOBROWSER_DOMAIN_ALLOWLIST` | *(unset)* | Comma-separated hosts (`github.com,*.docs.rs`); `navigate` rejects anything else |
| `NEOBROWSER_AUDIT` | `on` | `off` disables the append-only audit log (`~/.neobrowser/audit.log`, 0600, secrets masked) |
| `NEOBROWSER_REQUIRE_APPROVAL` | *(unset)* | Gate sensitive tools behind interactive approval (MCP elicitation): `1` = submit/form_fill/download/upload/login, or a comma-separated tool list |
| `NEOBROWSER_DISABLE_GPU` | *(unset)* | Force software rendering (GPU-less CI hosts only) |
| `NEOBROWSER_TOOLSET` | `core` | `core` advertises 26 tools; `full` advertises all 67. Tools outside the core set stay callable either way |
| `NEOBROWSER_SESSION_TTL_DAYS` | `30` | Lifetime of stored session material; `0` disables expiry |
| `NEOBROWSER_MAX_DOWNLOAD_MB` | `200` | Maximum download size |
| `NEOBROWSER_MAX_TABS` | `20` | Concurrent tab ceiling. Each tab is a renderer process |
| `NEOBROWSER_MAX_MEMORY_MB` | *(unset)* | Refuse to open more tabs once the browser tree exceeds this. Recommended for unattended agents |
| `NEOBROWSER_BRIDGE_PORT` | *(unset)* | Enable the [Chrome Bridge](#chrome-bridge-optional) on this loopback port |
| `NEOBROWSER_UPLOAD_DIR` | *(a default set)* | The **only** directory `upload` may read from. Recommended for unattended agents; otherwise Downloads, Desktop, Documents and the MCP roots the client declared |
| `NEOBROWSER_MAX_UPLOAD_MB` | `100` | Maximum size of a file `upload` will stage |
| `NEOBROWSER_HTTP_PORT` | *(unset)* | Enable the [MCP HTTP transport](#mcp-over-http-optional) on this port |
| `NEOBROWSER_HTTP_BIND` | `127.0.0.1` | Address the HTTP transport binds to. A non-loopback value is an explicit decision and warns on every start |
| `NEOBROWSER_INCLUDE_IDENTITY_COOKIES` | *(unset)* | **Risky escape hatch.** Setting it to `1` also imports Google/LinkedIn/Microsoft session-identity cookies, which those providers may flag as a duplicate session and log your real browser out. Off by default for that reason |
| `NEOBROWSER_VAULT_KEY` | *(unset)* | Base64 32-byte key, for hosts with no OS credential store (CI). Without it and without a keyring, session saves refuse rather than writing plaintext |
| `NEOBROWSER_LOG_FORMAT` | `text` | `json` emits structured logs carrying `trace_id` |
| `NEOBROWSER_CONFIG` | *(unset)* | Explicit config file path; see `neobrowser config init` |
| `NEOBROWSER_ALLOW_NO_SANDBOX` | *(unset)* | Last resort: run Chrome **without its sandbox**. `1` allows it; refused outright together with `NEOBROWSER_REAL_PROFILE` unless set to `with-real-profile`. Fix the host first — see [Sandbox](#sandbox) |
| `NEOBROWSER_POLICY` | `developer` | Policy profile: `developer`, `safe`, or `autonomous` — see [Policy](#policy) |
| `NEOBROWSER_ALLOW_DOMAINS` | *(unset)* | Comma-separated host suffixes the agent may reach. Once set it is **exclusive** — anything else is refused |
| `NEOBROWSER_DENY_DOMAINS` | *(unset)* | Host suffixes always refused, evaluated before the allowlist |
| `ANTHROPIC_API_KEY` | *(unset)* | Enables the optional LLM fallback in `find` (your key, your cost; off by default) |

## Policy

Every tool call is classified and evaluated before it runs, in one place, rather than each tool remembering its own guard. A refused call never reaches the browser.

```bash
# Interactive work (default): domain rules enforced, elevated actions logged
neobrowser doctor            # policy: developer

# Gate anything touching files, credentials or arbitrary script
NEOBROWSER_POLICY=safe

# Unattended agent: only these hosts, nothing else
NEOBROWSER_POLICY=autonomous NEOBROWSER_ALLOW_DOMAINS=example.com,api.example.com
```

| Profile | Domain rules | `js` / `upload` / `download` / auth |
|---|---|---|
| `developer` *(default)* | Enforced | Allowed, logged |
| `safe` | Enforced | `requires_confirmation` |
| `autonomous` | **Allowlist required** — an empty one refuses everything | Allowed only with a named, allowlisted destination |

Refusals are structured, so a model can adapt instead of retrying:

```json
{ "ok": false, "status": "blocked", "tool": "navigate", "action_class": "navigate",
  "reason": "internal.corp is on NEOBROWSER_DENY_DOMAINS",
  "remedy": "Choose a different destination, or remove the entry from NEOBROWSER_DENY_DOMAINS." }
```

Two deliberate choices worth knowing. `autonomous` treats a missing allowlist as "nothing is permitted" rather than "everything is", because an unattended agent with no boundary has no boundary. And the default is `developer`, not `safe`: most MCP clients do not implement elicitation, so asking for confirmation on ordinary actions would just be a profile that fails. Pick `safe` when a human is present to answer.

## Chrome Bridge (optional)

Three ways to give an agent a session, with honest trade-offs:

| Mode | What the agent gets | Cost |
|---|---|---|
| **Agent profile** (recommended) | A NeoBrowser profile you log into once | You log in once per site |
| **Bridge extension** | Tabs you share from your real browser | Manual install + per-tab consent |
| **Imported cookies** (advanced) | A copy of your Chrome session | Providers may flag the duplicate session |

```bash
NEOBROWSER_BRIDGE_PORT=9333 neobrowser serve   # then load extension/ in chrome://extensions
neobrowser bridge token                        # paste this into the extension popup
```

The bridge is authenticated with a per-session token in an `X-NeoBrowser-Token` header. That is not decoration: any web page you visit can reach `http://127.0.0.1`, and a `text/plain` POST is a "simple" request with no preflight — so without a custom header requirement, a hostile page could forge CDP results or drain the command queue. Requiring a custom header makes such a request impossible for a page to send at all.

`profile_mode` reports which mode is active and what it implies for your credentials.

## MCP over HTTP (optional)

stdio stays the primary local transport and nothing here changes it. The HTTP transport
exists for what stdio cannot serve: a container, a remote dev box, several clients against
one host.

```bash
NEOBROWSER_HTTP_PORT=8931 neobrowser serve
neobrowser http token                  # the bearer token to send
```

```bash
curl -X POST http://127.0.0.1:8931/mcp \
  -H "Authorization: Bearer $(neobrowser http token)" \
  -H "Mcp-Session-Id: my-client" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Three properties, each answering a real attack rather than a checkbox:

- **Authentication.** A bearer token on every request. Without it, anything that can reach
  the port drives a browser holding your sessions.
- **Origin validation.** A web page can POST to `127.0.0.1`, and with DNS rebinding a
  remote page can reach a LAN-bound port. `Origin` is compared by **exact host** — a prefix
  check would accept `http://localhost.evil.test`.
- **Session isolation.** Each `Mcp-Session-Id` gets its own browser, hence its own Chrome
  profile and cookies. Sharing one would hand session A's logged-in state to session B.
  Idle sessions are reaped, since each one holds a Chrome.

`DELETE /mcp` ends a session. Binding is loopback unless you override it, and a
non-loopback bind warns loudly on every start — put it behind a TLS proxy and treat the
token as a production credential.

## Sandbox

Chrome's renderer sandbox is **on by default**, and NeoBrowser refuses to launch without it rather than quietly disabling it. That matters more here than in most automation tools: the whole point is pointing a browser at arbitrary untrusted pages, so the sandbox is the boundary between a drive-by renderer exploit and your machine — and, in real-session mode, your logged-in accounts.

```bash
neobrowser doctor      # prints  sandbox: ON (host supports it)
```

If the host genuinely can't sandbox (running as root, or a Linux kernel with unprivileged user namespaces disabled), the launch fails with the specific blocker and how to fix it. `NEOBROWSER_ALLOW_NO_SANDBOX=1` exists as a last resort; it warns on every launch, and combining it with `NEOBROWSER_REAL_PROFILE` is refused unless you set it to the explicit `with-real-profile` value. Running as a non-root user is almost always the better answer.

## Security & responsible use

Real-session mode reads cookies from your Chrome profile and injects them into an automated browser. Treat it like any credential:

- It is **opt-in** — nothing touches your real profile unless you set `NEOBROWSER_REAL_PROFILE`.
- Chrome runs **sandboxed by default**; an unsandboxed run is explicit, logged, and blocked outright alongside real-profile cookies (see [Sandbox](#sandbox)).
- Cookie/session files are created `0600` under `~/.neobrowser` and written atomically, so they never exist world-readable even briefly. They are **not encrypted at rest** yet — a system-keychain vault is planned.
- Server-side fetches (`browse`, `download`) are **SSRF-guarded** to public http(s) only, and credentials are **origin-scoped**: cookies and any header outside a small content-negotiation allowlist are dropped the moment a redirect leaves the origin you asked for, including an `https → http` downgrade on the same host.
- The `login` tool refuses non-`https` URLs and never logs credentials.
- Anything an AI browses with your session acts **as you**. Point it only at sites and tasks you'd be comfortable doing yourself. This is a tool for automating *your own* accounts and workflows — not for evading access controls on services you don't own.
- **Automating a logged-in account may breach that service's terms.** Google, LinkedIn, X and others restrict automated access regardless of whose account it is, and enforcement lands on the account — rate-limiting, a challenge, or a ban. That risk is yours to weigh per site; NeoBrowser detects the wall, it does not indemnify you.

## Development

```bash
# Rust (primary):
cd rust && cargo test          # unit + live-Chrome + property/fuzz + embedded-JS (each self-skips
                               # when Chrome or Node is absent, rather than failing)
cargo test --test conformance                   # the Verified Action Contract scenarios
cargo test --test stealth_verify -- --ignored   # real bot.sannysoft detector

# Archived Python implementation (NOT the product; see archive/python-oracle/README.md):
cd archive/python-oracle && pip install -e ".[dev]" && python -m pytest -q
```

## License

MIT © Daniel Perez Pinazo
