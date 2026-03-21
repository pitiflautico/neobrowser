# PDR: End-to-End Browser Functionality

## Bug T1: RESOLVED

### Root cause
`bootstrap.js` uses `const` declarations. V8 doesn't allow re-declaring `const` in the same context. When `set_document_html()` re-executed bootstrap.js on re-navigation, all `const` statements silently failed → the old linkedom document stayed in place → all subsequent eval/extract returned stale content.

### Fix applied
Detect if runtime already initialized (`__neo_initialized` flag). On re-navigation, replace `document.documentElement.innerHTML` from a fresh `__linkedom_parseHTML()` call instead of re-running bootstrap.js. This preserves the linkedom environment (timers, fetch, shims) while swapping the page content.

### Layers verified
- `self.runtime` — same V8 isolate, reused across navigations ✅
- linkedom document — innerHTML replaced with new page content ✅
- html5ever DOM (`self.dom`) — re-parsed on each navigate() ✅
- WOM — re-extracted after DOM update ✅
- History stack — new entry pushed ✅
- Cookies — persisted in SQLite across navigations ✅

### What's still missing: PageContext invalidation model

## T1b: PageContext Model

Every navigation produces a new PageContext. All operations must execute against the current context. Stale references are invalid.

```rust
struct PageContext {
    page_id: u64,           // monotonic, incremented on each navigate()
    url: String,            // current page URL
    title: String,          // current page title
}
// dom_version removed: no mutation tracking yet, would be dead code
// runtime_generation removed: single runtime, no recreation
```

### Known limitation: innerHTML swap semantics
The re-navigation uses `document.documentElement.innerHTML = newHTML`. This is a CONSCIOUS trade-off:
- Inline `<script>` tags in new HTML are NOT re-executed (by spec, innerHTML doesn't run scripts)
- Old event listeners on replaced nodes are garbage collected (correct)
- Global JS state persists (window.*, vars) — this is intentional for SPA-like behavior
- To execute new page's scripts, `execute_page_scripts()` runs them explicitly after the swap

### Navigation failure modes
If re-navigation fails (timeout, DNS, HTTP 500):
- `process_pending_navigations()` prints error to stderr
- page_id does NOT increment (page stays on previous content)
- No retry — caller must re-trigger navigation
- Retry strategy is caller's responsibility (MCP tool / REPL / agent)

### Invalidation contract
After `navigate()`, `process_pending_navigations()`, or any action that triggers full-document navigation:
- `page_id` increments
- All prior selector results are invalid (re-resolve required)
- WOM cache invalidated
- Active frame reset to top
- LiveDom dispatcher re-injected (if needed)

### Freshness assertions
Every `extract()`, `eval()`, `click()`, `type_text()` should:
1. Record `page_id` at start of operation
2. If `page_id` changed during operation → return `StalePageContext` error
3. Return `page_id` in result so caller can detect staleness

Implementation: add `page_id: AtomicU64` to NeoSession, increment in `navigate()`.

## T2: Form Benchmark

Stable sites only. No Google (consent), no ChatGPT (anti-bot). 4 metrics per site.

| Site | Form | Action | M1: Fields filled | M2: Submit fired | M3: Page changed | M4: Expected content |
|------|------|--------|-------------------|------------------|-------------------|----------------------|
| httpbin.org/forms/post | POST form | Fill custname+custtel+comments | eval: input.value == expected | navigation request captured | page_id incremented + URL = /post | Response body contains submitted values |
| httpbin.org/get | GET via URL | Navigate with query params | N/A | N/A | page loaded | JSON response with args |
| HN login | Login form | Fill acct+pw, submit | eval: input.value set | navigation triggered | URL changed | Response contains "Bad login" text |
| DuckDuckGo HTML | Search form | Fill q, submit | eval: input.value set | navigation triggered | URL contains q= param | Results page has >0 result links |
| example.com | No form (baseline) | Navigate only | N/A | N/A | page loaded | "Example Domain" in title |

### Metrics definition
- **M1 (Fields filled)**: `eval("document.querySelector(sel).value")` returns expected text
- **M2 (Submit fired)**: `drain_navigation_requests()` returns non-empty after submit
- **M3 (Page changed)**: `page_id` after > `page_id` before, AND `document.title` or URL differs
- **M4 (Expected content)**: `eval` or `extract_text` contains expected string/pattern

### Pass criteria
- 4/5 sites pass all applicable metrics
- page_id correctly tracks navigations
- No stale content in any extract after navigation

## T3: Multi-Page Session

Test cookie + session persistence across navigations.

```
1. Navigate to httpbin.org/cookies/set/testcookie/neorender_v2
   → Assert: response confirms cookie set
2. Navigate to httpbin.org/cookies
   → Assert: response shows testcookie=neorender_v2
3. Navigate to httpbin.org/get
   → Assert: Cookie header contains testcookie=neorender_v2
```

Observable assertions:
- Cookie visible in `document.cookie` after step 1
- Cookie sent in request headers in step 2 (visible in httpbin response)
- Cookie persists across 3 navigations to different paths

## T4: MCP Integration Test

Test all 8 tools in sequence WITH navigation between calls. Verify no stale context.

```
1. browse("https://httpbin.org/forms/post") → WOM with inputs
2. interact(type, "input[name=custname]", "Claude") → ok
3. eval("document.querySelector('input[name=custname]').value") → "Claude"
4. interact(submit, "form") → navigation triggered
5. extract(text) → contains submitted data (page_id must be new)
6. browse("https://news.ycombinator.com") → WOM with HN content (not httpbin)
7. extract(text) → contains "Hacker News" (not httpbin)
8. wait("table", 5000) → found
```

Key assertions:
- Step 5: extract returns NEW page content, not httpbin form
- Step 6-7: after browse to different site, no residual httpbin content
- page_id different at steps 1, 5, and 6

## T5: ChatGPT Pong (EXTERNAL — not gate)

Stress test only. Not blocking. Documents what works and what doesn't.

## Gate

### Core (blocking)
- PageContext model implemented with page_id
- 4/5 form benchmark sites pass all metrics
- Multi-page cookie test passes (3 navigations, cookie persists)
- MCP sequence test passes (8 steps, no stale content)
- All ~338 existing tests pass
- page_id increments correctly on every navigation

### External (non-blocking)
- ChatGPT pong
- Mercadona.es SPA loading
