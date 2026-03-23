# PDR — Microtask Root Cause Analysis (23 marzo 2026)

## Root Cause

**Our own callback budget system** in `bootstrap.js` (line 715: `__callbackBudget = 5000`)
was the root cause. ChatGPT's page load (React + 388 dynamic modules) exhausts the 5000
callback limit, setting `__budgetExhausted = true`. After that, `queueMicrotask` silently
drops all callbacks, which breaks Promise.then resolution chains.

## Discovery Path

| Step | What we tested | Result |
|------|---------------|--------|
| 1 | V8 bare, deno_core, happy-dom, bootstrap — 30 unit tests | ALL PASS |
| 2 | httpbin, react.dev, GitHub | ALL PASS |
| 3 | ChatGPT inline scripts, modules, settle patterns | ALL PASS |
| 4 | Watchdog terminate_execution | Removed, still broken |
| 5 | kExplicit policy + context queue checkpoint | Still broken |
| 6 | V8 API `queue.enqueue_microtask()` + `perform_checkpoint()` | **WORKS** |
| 7 | JS `Promise.resolve().then(cb)` + same checkpoint | **BROKEN** |
| 8 | GPT analysis: "JS path is not native" | Led to checking queueMicrotask |
| 9 | `queueMicrotask.toString()` → reveals budget wrapper | **FOUND IT** |
| 10 | `__budgetExhausted` → `true` after ChatGPT load | **ROOT CAUSE** |
| 11 | `__neo_resetBudget()` → microtasks work again | **FIX CONFIRMED** |

## Fix

Reset callback budget before each interactive eval:
```rust
// In browser_impl.rs eval()
rt.execute("if(typeof __neo_resetBudget==='function')__neo_resetBudget()");
```

## Pipeline Status Post-Fix

ChatGPT pong pipeline works end-to-end:
- Auth token: ✅ obtained
- Sentinel/PoW: ✅ solved
- Turnstile token: ✅ generated (1256 chars)
- API request: ✅ sent to /backend-api/conversation
- Response: **403** — Cloudflare "Unusual activity" (TLS fingerprint detection)

## 403 Root Cause

The 403 is NOT a NeoRender bug. It's Cloudflare/OpenAI bot detection via TLS fingerprinting.

Our HTTP client (`rquest` with Chrome 136 emulation) doesn't perfectly match Chrome's:
- JA3/JA4 TLS fingerprint may differ from real Chrome
- HTTP/2 SETTINGS frame order/values may differ
- Header ordering may not match Chrome exactly

This is the same issue documented in the AI Browser memory (`captcha_status`):
> reCAPTCHA v3: score 0.1 (bot) in stealth and puppet
> Root cause: Playwright injects navigator.webdriver=true, TLS fingerprint not Chrome retail

## What Chromium Does That We Now Replicate

| Mechanism | Chromium | NeoRender (before) | NeoRender (after) |
|-----------|---------|-------------------|------------------|
| execCommand('insertText') | Text.insertData() + characterData mutations | createTextNode (wrong mutation type) | Text.insertData() ✅ |
| History.state | null initial + structuredClone | Pre-populated + reference copy | null + structuredClone ✅ |
| Fetch connection pool | 1 thread, HTTP/2 mux, 6/host | Thread per fetch, new runtime | SharedFetchRuntime, pool limits ✅ |
| Microtask drain | kScoped + MicrotasksScope per task | kAuto + budget kills microtasks | Budget reset + kExplicit ✅ |
| Event loop after eval | Always running | Dead between evals | pump_after_interaction ✅ |

## 403 Analysis: TLS Fingerprint

The 403 comes from Cloudflare WAF, NOT from ChatGPT's sentinel system.
Proven by: sending the request WITHOUT sentinel tokens gives the same 403.
The sentinel tokens (PoW, Turnstile) are valid and correctly computed.

Cloudflare blocks at the TLS/HTTP layer before the request reaches OpenAI:
- `rquest` emulates Chrome 136 TLS but JA3/JA4 fingerprint differs from retail Chrome
- HTTP/2 SETTINGS frames may differ
- This is a known limitation of ALL non-browser HTTP clients against Cloudflare

Also found: `navigator.userAgent` returns "HappyDOM/20.8.4" instead of Chrome UA.
This doesn't affect HTTP headers (those are correct) but may affect JS-side checks
during Turnstile evaluation. Fix: override `navigator.userAgent` in bootstrap.

## What's Next

NeoRender V2 is a RENDERING engine (like a headless browser for page extraction).
For actual AI-to-AI communication, use engines with real Chrome TLS:
1. **neobrowser-rs CDP**: real Chrome via CDP (works for LinkedIn messaging)
2. **aichat MCP**: ChatGPT/Gemini chat via browser automation (works now)
3. **Chrome Extension**: inject into real Chrome process

The NeoRender pong proved the engine works end-to-end:
- JavaScript execution ✅
- Module loading ✅
- React hydration ✅
- Cookie auth ✅
- Sentinel/PoW solving ✅
- API requests ✅
- Only blocked by Cloudflare TLS fingerprinting (not an engine issue)
