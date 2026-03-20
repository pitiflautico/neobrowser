# NeoRender Browser — What We Learned

## Architecture

NeoRender is a browser without a rendering engine: V8 + linkedom + rquest.

| Layer | Tech | Role |
|---|---|---|
| HTTP | rquest (Chrome 136 TLS) | Network requests with real Chrome fingerprint |
| DOM | linkedom | HTML parsing + DOM API |
| JS | deno_core (V8) | Full JavaScript execution |
| Layout | layout.js stubs | Fake but realistic dimensions for fingerprint checks |
| Chrome | CDP fallback | Only for sites that need Turnstile/captcha |

**37K lines** — 30 JS modules, 17 Rust modules.

## Cloudflare Detection — What We Proved

### TLS Fingerprint is Everything

The #1 detection vector is the TLS fingerprint (JA3/JA4 hash). Cloudflare compares it against the User-Agent header.

| rquest version | Chrome impersonation | Cloudflare result |
|---|---|---|
| 1.5 (Chrome 131) | Old TLS | **BLOCKED** on 5/20 sites |
| 5.1 (Chrome 136) | Current TLS | **PASS** on 19/20 sites |

**Fix**: Upgrade rquest to 5.1 with `Emulation::Chrome136`. This single change went from 15/20 to 19/20 sites passing.

### Headers Must Match a Real Chrome

Missing or wrong headers trigger detection:

| Header | Required for | Value |
|---|---|---|
| `Sec-Ch-Ua` | All sites | `"Chromium";v="136", "Not_A Brand";v="24", "Google Chrome";v="136"` |
| `Sec-Ch-Ua-Platform` | All sites | `"macOS"` |
| `Sec-Fetch-Dest` | Navigation | `document` |
| `Sec-Fetch-User` | Navigation | `?1` |
| `Upgrade-Insecure-Requests` | Navigation | `1` |
| `Accept` | Navigation | `text/html,application/xhtml+xml,...` |
| `Accept-Encoding` | All | `gzip, deflate, br, zstd` |

Two separate header sets: navigation (full) vs fetch/XHR (lighter).

### CDP Modifications Cause Detection

Cloudflare detects Chrome DevTools Protocol modifications:

| CDP action | Detection | Notes |
|---|---|---|
| `Page.addScriptToEvaluateOnNewDocument` | **YES** | Cloudflare checks for this |
| `Network.setUserAgentOverride` | **YES** | Changes fingerprint mid-session |
| `Runtime.evaluate` (post-navigation) | Usually OK | After initial load |
| `Page.enable` + `Runtime.enable` | OK | Required for basic CDP |

**Rule**: Launch Chrome 100% clean. Zero stealth patches. Only use CDP for reading DOM and dispatching events.

### Headless Modes

| Mode | Cloudflare | Visible on macOS |
|---|---|---|
| `--headless=new` | **BLOCKED** (different TLS) | No |
| `--window-position=-32000,-32000` (offscreen) | **PASS** | Dock only |
| No flags (headed) | **PASS** | Yes |
| Stealth pipe + patches | **INTERMITTENT** | No |

**Best**: Offscreen (`--window-position=-32000`) with zero CDP modifications. Test confirms headed = headless parity (15/15 nodes, 0 WAF blocks).

## ChatGPT API — Anatomy of a Message

### Endpoint

`POST https://chatgpt.com/backend-api/f/conversation`

(NOT `/backend-api/conversation` — the `/f/` prefix is required)

### Required Headers

| Header | Source | TTL | Via rquest? |
|---|---|---|---|
| `Authorization` | `/api/auth/session` | ~12h | ✅ |
| `OpenAI-Sentinel-Chat-Requirements-Token` | `/backend-api/sentinel/chat-requirements` | ~9min | ✅ |
| `OpenAI-Sentinel-Turnstile-Token` | Cloudflare Turnstile JS widget | ~60s | ❌ needs browser |
| `OpenAI-Sentinel-Proof-Token` | Client-side proof-of-work JS | ~60s | ❌ needs browser |
| `x-conduit-token` | ChatGPT JS runtime | ~60s | ❌ needs browser |
| `OAI-Device-Id` | UUID in cookies/localStorage | Persistent | ✅ |
| `OAI-Client-Build-Number` | Page HTML | Per deploy | ✅ |
| `Content-Type` | Static | — | ✅ |
| `Accept` | Static (`text/event-stream`) | — | ✅ |

### Request Body

```json
{
  "action": "next",
  "messages": [{
    "id": "uuid",
    "author": {"role": "user"},
    "create_time": 1773999526.0,
    "content": {"content_type": "text", "parts": ["message"]},
    "metadata": {}
  }],
  "model": "auto",
  "timezone_offset_min": -60,
  "timezone": "Europe/Madrid",
  "conversation_mode": {"kind": "primary_assistant"},
  "supports_buffering": true,
  "parent_message_id": "uuid"
}
```

Remove `"supported_encodings": ["v1"]` to get raw SSE (not delta-encoded).

### Response

SSE stream. Assistant response in events with `message.author.role == "assistant"` and `message.content.parts[0]`.

## Token Harvest + rquest Replay (The Working Approach)

### Flow

```
Chrome (offscreen) → generates fresh tokens → killed
rquest (Chrome 136 TLS) → sends POST with harvested tokens → GPT responds
```

### How It Works

1. Chrome offscreen opens ChatGPT (uses persistent profile with cookies)
2. JS fetch interceptor injected: captures headers when ChatGPT JS calls fetch
3. `send_message` triggers ChatGPT to generate all tokens (turnstile, proof, conduit)
4. Interceptor captures the full Headers object (forEach)
5. Tokens extracted via CDP eval
6. Chrome killed
7. rquest sends POST with captured headers + custom message body
8. GPT responds via SSE

### Performance

| Phase | Time |
|---|---|
| Chrome harvest (open + send + capture) | ~24s |
| rquest replay (POST + GPT response) | ~13s |
| **Total** | **~37s** |

### Key Detail: Interceptor Code

```javascript
window.__neo_tokens = null;
const _f = window.fetch;
window.fetch = async function(input, opts) {
    const url = typeof input === 'string' ? input : input?.url || '';
    if (url.includes('/f/conversation')) {
        const h = opts?.headers || {};
        const m = {};
        if (h.forEach) h.forEach((v,k) => m[k] = v);
        else if (typeof h === 'object')
            Object.entries(h).forEach(([k,v]) => m[k] = String(v));
        window.__neo_tokens = { h: m };
        // Forward to original — Chrome sends normally
        return _f.apply(this, arguments);
    }
    return _f.apply(this, arguments);
};
```

Captures tokens AND forwards to original fetch (Chrome sends the message too, which keeps the session alive).

## React Hydration in V8 — What We Tried

ChatGPT uses React Router 7.13.1 (not Next.js). Hydration requires:

1. ✅ 47 ES modules loaded (390KB manifest + 3.5MB vendor + route modules)
2. ✅ modulepreload links pre-fetched to store
3. ✅ On-demand module fetching for missing imports
4. ✅ ViewTransition API polyfill (React 19)
5. ❌ `document.startViewTransition().finished.then()` — React calls .then() on null

The blocker: React Router's SSR streaming hydration reads a ReadableStream embedded in the HTML. Our V8 executes the modules but the hydration pipeline crashes with `TypeError: Cannot read properties of null (reading 'then')`. Root cause is in React's internal Suspense/lazy loading, not in our code.

**Verdict**: Full React hydration in V8 is possible but requires more DOM API fidelity. Parking for now — the token harvest approach works.

## NeoRender Results (V8 browser, no Chrome)

| # | Site | Result | KB |
|---|---|---|---|
| 1 | HN | ✅ | 33 |
| 2 | Google | ✅ | 354 |
| 3 | Reddit | ✅ | 567 |
| 4 | YouTube | ✅ | 723 |
| 5 | Wikipedia | ✅ | 1076 |
| 6 | Amazon | ✅ | 1017 |
| 7 | Stack Overflow | ✅ | 234 |
| 8 | ChatGPT | ✅ (read) | 281 |
| 9 | NYT | ✅ | 1241 |
| 10 | BBC | ⏱ timeout | — |
| 11 | El País | ✅ | 431 |
| 12 | Apple | ✅ | 229 |
| 13 | Microsoft | ✅ | 225 |
| 14 | Netflix | ✅ | 538 |
| 15 | Instagram | ✅ | 648 |
| 16 | Notion | ✅ | 279 |
| 17 | Google Docs | ✅ | 1177 |
| 18 | Twitch | ✅ | 182 |
| 19 | Facebook | ✅ | 401 |
| 20 | LinkedIn | ✅ | 8757 |

**19/20 without any Cloudflare block** (was 15/20 with Chrome 131 TLS).

## Tools Added This Session

| Tool | Purpose |
|---|---|
| `browser_fetch` | HTTP via rquest (Chrome 136 TLS), no Chrome |
| `browser_record` | Traffic recorder (monkey-patches fetch/XHR in Chrome) |

## Files Changed

| File | Change |
|---|---|
| `Cargo.toml` | rquest 1.5 → 5.1, added rquest-util |
| `src/neorender/net/mod.rs` | Chrome 136 headers, navigation vs fetch sets |
| `src/neorender/session.rs` | modulepreload, dynamic scripts, inline modules |
| `src/neorender/v8_runtime.rs` | On-demand module fetching, error recovery |
| `src/neorender/mod.rs` | modulepreload extraction, preload_only flag |
| `src/engine.rs` | Zero CDP modifications, offscreen mode |
| `src/mcp.rs` | browser_fetch, browser_record, headless config |
| `src/ghost.rs` | Chrome 136 headers, call_api_with_headers |
| `js/layout.js` | Element dimensions, canvas, WebGL, ViewTransition |
| `js/dynamic_scripts.js` | appendChild interceptor for dynamic scripts |
| `js/bootstrap.js` | ViewTransition polyfill note |
| `tests/test_headless_parity.sh` | Headed vs headless comparison test |
