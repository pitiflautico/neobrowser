# Cross-tool bot-detection study: NeoBrowser vs Playwright MCP

## Methodology

- Date: 2026-08-19 12:08:16 +0200 · machine: `Darwin arm64` · single machine, single IP, no proxies.
- Runs per cell: **N=2** (each run = fresh server process + fresh browser).
- NeoBrowser: `0.1.7` (real Chrome via CDP, `NEOBROWSER_HOME=/tmp/nb-study`).
- Playwright MCP: `npx -y @playwright/mcp@latest --headless` (serverInfo: Playwright 1.63.0-alpha-2026-08-05).
- Both tools were driven by the SAME harness with IDENTICAL JS blobs per target (same pattern as `bench/compare.py`). Wall classification uses one shared regex set.
- `access` is defined per target: sannysoft = results table parsed (>=5 rows); creepjs = page rendered (>500 chars of body text); nowsecure = real content reached (no captcha/challenge wall); deviceandbrowserinfo = info page reached (the page does not always render a webdriver row — when absent we report `navigator.webdriver` from the page's own JS context instead, and say so).
- Latency = wall time of the navigate call only (server startup excluded).

## Results

| target | tool | run | access | wall | sannysoft pass/fail | webdriver | latency ms | error |
|---|---|---|---|---|---|---|---|---|
| sannysoft | NeoBrowser | 1 | ✅ | none | 11/0 | undefined | 4755 |  |
| sannysoft | NeoBrowser | 2 | ✅ | none | 11/0 | undefined | 4241 |  |
| sannysoft | Playwright MCP (headless) | 1 | ✅ | none | 10/1 | false | 1220 |  |
| sannysoft | Playwright MCP (headless) | 2 | ✅ | none | 10/1 | false | 973 |  |
| creepjs | NeoBrowser | 1 | ✅ | none | — | undefined | 3159 |  |
| creepjs | NeoBrowser | 2 | ✅ | none | — | undefined | 3041 |  |
| creepjs | Playwright MCP (headless) | 1 | ✅ | none | — | false | 551 |  |
| creepjs | Playwright MCP (headless) | 2 | ✅ | none | — | false | 511 |  |
| nowsecure | NeoBrowser | 1 | ❌ | captcha | — | undefined | 3610 |  |
| nowsecure | NeoBrowser | 2 | ❌ | captcha | — | undefined | 3344 |  |
| nowsecure | Playwright MCP (headless) | 1 | ❌ | captcha | — | false | 744 |  |
| nowsecure | Playwright MCP (headless) | 2 | ❌ | captcha | — | false | 639 |  |
| deviceandbrowserinfo | NeoBrowser | 1 | ✅ | none | — | undefined | 3197 |  |
| deviceandbrowserinfo | NeoBrowser | 2 | ✅ | none | — | undefined | 3147 |  |
| deviceandbrowserinfo | Playwright MCP (headless) | 1 | ✅ | none | — | false | 650 |  |
| deviceandbrowserinfo | Playwright MCP (headless) | 2 | ✅ | none | — | false | 572 |  |

## Sannysoft per-check detail (run 1)

**NeoBrowser**

| check | status | result |
|---|---|---|
| User Agent (Old) | pass | User Agent (Old) / Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0 |
| WebDriver (New) | pass | WebDriver (New) / missing (passed) |
| WebDriver Advanced | pass | WebDriver Advanced / passed |
| Chrome (New) | pass | Chrome (New) / present (passed) |
| Permissions (New) | pass | Permissions (New) / default |
| Plugins Length (Old) | pass | Plugins Length (Old) / 5 |
| Plugins is of type PluginArray | pass | Plugins is of type PluginArray / passed |
| Languages (Old) | pass | Languages (Old) / es-ES,es |
| WebGL Vendor | pass | WebGL Vendor / Google Inc. (Apple) |
| WebGL Renderer | pass | WebGL Renderer / ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version) |
| Broken Image Dimensions | pass | Broken Image Dimensions / 16x16 |

**Playwright MCP (headless)**

| check | status | result |
|---|---|---|
| User Agent (Old) | fail | User Agent (Old) / Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome |
| WebDriver (New) | pass | WebDriver (New) / missing (passed) |
| WebDriver Advanced | pass | WebDriver Advanced / passed |
| Chrome (New) | pass | Chrome (New) / present (passed) |
| Permissions (New) | pass | Permissions (New) / prompt |
| Plugins Length (Old) | pass | Plugins Length (Old) / 5 |
| Plugins is of type PluginArray | pass | Plugins is of type PluginArray / passed |
| Languages (Old) | pass | Languages (Old) / es-ES,es |
| WebGL Vendor | pass | WebGL Vendor / Google Inc. (Apple) |
| WebGL Renderer | pass | WebGL Renderer / ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version) |
| Broken Image Dimensions | pass | Broken Image Dimensions / 16x16 |


## Reading the numbers (honestly)

- **Sannysoft:** NeoBrowser passed all 11 checks in both runs. Playwright MCP failed `User Agent (Old)` in both runs — its UA string contains `HeadlessChrome`, a direct consequence of the `--headless` config used here.
- **nowsecure.nl (Cloudflare): BOTH tools were blocked in BOTH runs** (challenge page, classified `captcha`). NeoBrowser's stealth fingerprint did not get it through a real Cloudflare wall from this IP. No bypass claim is made anywhere in this study.
- **Latency:** Playwright MCP navigated 3-5x faster on every target. Consistent with `bench/compare.py`, this is NeoBrowser's deliberate frame-forcing (`nudge_frame`) so deferred content actually renders — a correctness-over-speed trade-off, disclosed as such.
- **`navigator.webdriver`:** NeoBrowser reads `undefined` (property absent from the page's JS context) and Playwright headless reads `false`. A stock non-automated Chrome reads `false`, so `undefined` is itself atypical — neither test site flagged it, but a stricter detector could. Reported as observed.
- **CreepJS** loaded for both tools, but no trust score was present in the DOM at read time in any of the 4 cells — reported as 'not read', not as pass or fail.
- **deviceandbrowserinfo.com/info** rendered for both tools but showed no webdriver row in any run; the `webdriver` column above comes from evaluating `navigator.webdriver` directly.

## What this does NOT prove

- Single machine, single datacenter/residential IP, **no proxy rotation** — real-world walls are IP-reputation-driven as much as fingerprint-driven.
- **N=2 per cell** — enough to show the harness works and variance is small/large, not enough for statistical claims.
- Public test sites (sannysoft, creepjs, nowsecure) are **proxies for** bot detection, not the walls of real protected sites; passing here does not imply bypassing production defenses.
- Playwright MCP was run **headless** (`--headless`, matching `bench/compare.py`); a headed Playwright run could score differently. NeoBrowser ran its default real-Chrome config.
- CreepJS was read a fixed number of seconds after load; its full analysis may not have finished, so trust-score absence is reported as 'not read', not as a fail.
