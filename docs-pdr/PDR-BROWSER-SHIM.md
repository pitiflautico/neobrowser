# PDR: Browser Compatibility Shim — Partial Semantics over linkedom

## What This Is
A compatibility shim that gives linkedom enough browser-like behavior for AI agent use cases: navigate pages, fill forms, extract content, maintain sessions. NOT a real browser. Sites that depend on real layout, real CSS, real rendering will NOT work.

## What This Is NOT
- Not a layout engine (no real positions, sizes, or CSS cascade)
- Not a stealth tool (fingerprint coherence baseline, not guarantee)
- Not complete browser semantics (partial, documented gaps)

## Architecture
```
Site JS code
    ↓ calls browser APIs
browser_shim.js (intercepts)
    ↓ calls Deno.core.ops
Rust ops (real implementations where possible, stubs where not)
    ↓
Engine state (navigation queue, cookie store, history stack)
```

## API Classification Matrix

Every API falls into one of four categories:

| Category | Meaning | Example |
|----------|---------|---------|
| **SEMANTIC** | Real behavior, correct results | form.submit() → HTTP request |
| **COMPAT** | Prevents crash, partial correctness | matchMedia → always desktop |
| **STUB** | Returns fake data, no real behavior | getBoundingClientRect → zeros |
| **EXCLUDED** | Not implemented, documented gap | WebSocket, Service Workers |

---

## TIER 1: Navigation (SEMANTIC)

### Navigation Model

Three distinct navigation types:

| Type | Trigger | Engine behavior |
|------|---------|-----------------|
| **Full document** | form.submit(), location.assign(), location.replace() | HTTP request → new HTML → re-parse → re-execute JS |
| **Same-document** | history.pushState(), hash change | Update URL + history stack, dispatch popstate, NO HTTP |
| **External** | window.open(), target="_blank" | Detect → signal NewContext → do NOT follow |

### form.submit() — SEMANTIC

Full implementation, not just FormData serialization:

```
1. Determine submitter (button that triggered, or null for .submit())
2. If submitter has formaction/formmethod/formenctype → override form attributes
3. Resolve action URL (relative to document.baseURI, not just location)
4. Determine method: form.method || 'GET' (default)
5. Determine enctype:
   - 'application/x-www-form-urlencoded' (default)
   - 'multipart/form-data' (if enctype set or file inputs)
   - 'text/plain' (rare)
6. Collect form data:
   - Skip disabled fields
   - Skip unchecked radio/checkbox
   - Include selected <option> values
   - Include <textarea> content
   - Handle multiple select
   - Include submitter name=value if submitter has name
7. If method=GET: append as query string
   If method=POST: encode as body per enctype
8. Do NOT run constraint validation (.submit() bypasses it)
   Note: submit via Enter or button click DOES validate — different path
9. Determine target: form.target || submitter.formtarget || '_self'
   If _blank/_new → NewContext signal, don't navigate
10. Queue navigation request to Rust
```

### <a>.click() — SEMANTIC

Not all clicks navigate:

```
1. If event.defaultPrevented → do nothing (SPA routers call preventDefault)
2. If modifier keys (ctrl/meta/shift) → would open new tab → NewContext signal
3. If a.target === '_blank' → NewContext signal
4. If a.download attribute → signal download intent, don't navigate
5. If href starts with '#' → same-document navigation (update hash only)
6. If href starts with 'javascript:' → eval the JS, don't navigate
7. If href starts with 'mailto:' / 'tel:' → signal, don't navigate
8. Otherwise → full document navigation
```

### location — SEMANTIC

```javascript
// Full Location interface with URL parsing
__neo_location = new URL(currentUrl);
// Setters trigger navigation:
//   .href = url     → full navigation
//   .pathname = p   → full navigation to origin + p
//   .search = s     → full navigation with new query
//   .hash = h       → same-document (hash change only)
// .assign(url) → full navigation (adds to history)
// .replace(url) → full navigation (replaces history entry)
// .reload() → full navigation to current URL
```

### Base URL Resolution

All relative URLs (form action, link href, script src) resolved against:
1. `<base href="...">` if present
2. Otherwise `location.href`
This MUST be correct or SPAs with base tags break.

---

## TIER 2: State (SEMANTIC)

### History — Three models

```
enum HistoryNavigation {
    Synthetic,        // pushState/replaceState — JS only, no HTTP
    SameDocument,     // hash change — JS only, dispatch hashchange
    FullDocument,     // back/forward to different URL — HTTP + reload
}
```

Implementation:
- Internal stack of `{url, state, title}` entries
- `pushState(state, title, url)` → add entry, update location, NO navigation
- `replaceState(state, title, url)` → replace current, update location, NO navigation
- `back()` → check stored entry type: if entry was created by pushState/replaceState → synthetic (dispatch popstate). If entry was created by full navigation → queue full document navigation. Decision based on entry metadata, NOT URL heuristics.
- `forward()` → same logic as back()
- Each history entry stores: `{url, state, title, nav_type: Synthetic|FullDocument}`
- `history.length` → stack length
- `history.state` → current entry's state

### document.cookie — SEMANTIC (not Set-Cookie header semantics)

`document.cookie = "a=b"` is NOT Set-Cookie header format. Differences:
- No HttpOnly (JS can't set HttpOnly cookies)
- Attributes: name=value; path=/; domain=.x.com; expires=Date; max-age=N; secure; samesite=Lax
- Omitted attributes → defaults (path=current, domain=current, session cookie)
- Setting same name+path+domain → overwrites
- Reading → returns all non-HttpOnly cookies as "name=value; name2=value2"

Implementation:
```
op_cookie_get(origin) → filter cookies by origin, skip HttpOnly, format as string
op_cookie_set(origin, cookie_string) → parse name=value + attributes → insert into SqliteCookieStore
```

---

## TIER 3: Layout & Visibility (COMPAT — prevents crashes, NOT real layout)

**Honest limitation**: These shims prevent JS exceptions. They do NOT enable correct layout-dependent behavior. Sites using virtualized lists, sticky headers, popovers based on position, or IntersectionObserver for meaningful visibility logic WILL behave incorrectly.

| API | Category | Behavior | Known failures |
|-----|----------|----------|----------------|
| `IntersectionObserver` | COMPAT | Report all as visible, ratio=1. Triggers callback once. | May over-trigger lazy loading (memory spike), breaks virtualized lists |
| `ResizeObserver` | COMPAT | Report 1920x1080 once | Breaks responsive JS that actually checks sizes |
| `matchMedia` | COMPAT | Assume desktop (min-width:1024 matches, prefers-color-scheme:light) | Wrong for mobile-first sites |
| `getComputedStyle` | STUB | Returns {display:'block', visibility:'visible', opacity:'1', position:'static'} | Breaks anything checking real styles |
| `getBoundingClientRect` | STUB | Returns {top:0, left:0, width:0, height:0} (ZEROS, not fake positions) | Breaks position-dependent code, but zeros are less dangerous than fake numbers |
| `offsetWidth/Height` | STUB | Returns 0 | Triggers "element is hidden" heuristics in some frameworks |
| `scrollIntoView` | STUB | No-op, record intent | Scroll-dependent loading won't trigger |
| `window.innerWidth/Height` | COMPAT | 1920x1080 | Fixed, non-responsive |
| `screen.*` | COMPAT | 1920x1080 | Fixed |
| `devicePixelRatio` | COMPAT | 1 | Won't trigger retina paths |

### Capability boundary (EXPLICIT)
These classes of sites will NOT work correctly with fake layout:
- Virtualized lists (react-virtualized, react-window)
- Infinite scroll based on scroll position
- Sticky headers / position:fixed dependent on scroll
- Popover/tooltip positioning
- Drag and drop
- Canvas/WebGL rendering

---

## TIER 4: Document State (COMPAT/SEMANTIC mix)

| API | Category | Behavior |
|-----|----------|----------|
| `document.readyState` | COMPAT | 'loading' → 'interactive' → 'complete' (set by engine at pipeline phases, not true browser load cycle — strong compat, not full semantic) |
| `document.hidden` | COMPAT | false |
| `document.visibilityState` | COMPAT | 'visible' |
| `document.hasFocus()` | COMPAT | true |
| `document.referrer` | SEMANTIC | Set from previous navigation URL |
| `document.title` | SEMANTIC | Read/write from DOM |
| `document.baseURI` | SEMANTIC | Computed from <base> tag or location |
| `navigator.userAgent` | COMPAT | Chrome 136 UA (emulated — must match rquest TLS fingerprint for coherence, not truth of environment) |
| `navigator.language` | COMPAT | 'en-US' |
| `navigator.cookieEnabled` | COMPAT | true |
| `navigator.onLine` | COMPAT | true |
| `navigator.webdriver` | SEMANTIC | false (CRITICAL — never true) |
| `navigator.hardwareConcurrency` | COMPAT | 8 |
| `navigator.maxTouchPoints` | COMPAT | 0 |

### MutationObserver — RISK ITEM
linkedom's MutationObserver support is partial. Before shipping:
1. Test: does it fire on setAttribute? YES/NO
2. Test: does it fire on textContent change? YES/NO
3. Test: does it fire on appendChild/removeChild? YES/NO
4. Test: does it respect subtree/childList/attributes config? YES/NO

If any critical test fails → implement our own on top of linkedom DOM mutations.
This is a **P0 gate blocker** for Phase 2. Phase 2 CANNOT close without MutationObserver verified or replaced. It blocks half of modern framework compatibility.

---

## TIER 5: Events & Timing (COMPAT)

| API | Category | Behavior |
|-----|----------|----------|
| `document.activeElement` | SEMANTIC | Track via focus/blur interception |
| `el.focus()` / `el.blur()` | SEMANTIC | Update activeElement + dispatch focus/blur/focusin/focusout events |
| `requestAnimationFrame` | COMPAT | Execute callback ONCE with performance.now() timestamp. NOT animation support — crash prevention only. |
| `cancelAnimationFrame` | STUB | No-op |
| `requestIdleCallback` | COMPAT | Execute callback with {timeRemaining: () => 50, didTimeout: false} |
| `performance.now()` | SEMANTIC | Monotonic clock from Rust Instant::now() |
| `performance.mark/measure` | COMPAT | Store in memory |
| `crypto.getRandomValues` | SEMANTIC | Real random via Rust |
| `crypto.randomUUID` | SEMANTIC | Real UUID |

---

## TIER 6: Network Compat (COMPAT/STUB)

NOT "Network & Communication" — most of this is crash prevention.

| API | Category | Behavior |
|-----|----------|----------|
| `XMLHttpRequest` | COMPAT | Route through op_fetch. Sync mode EXCLUDED. Must implement: readystatechange, load, error, abort events + readyState transitions (0→1→2→3→4) + responseText/status/statusText. Without events, "request completes" is untestable. |
| `EventSource` / SSE | STUB | Constructor doesn't crash. No real connection. |
| `WebSocket` | EXCLUDED | Constructor throws "WebSocket not available in NeoRender" |
| `navigator.sendBeacon` | STUB | No-op, return true |
| `AbortController/Signal` | SEMANTIC | Real implementation (needed for React fetch) |
| `Blob/File/FileReader` | COMPAT | Basic implementations for form upload |
| `atob/btoa` | SEMANTIC | Real base64 (if missing in V8 context) |

---

## TIER 7: Permissions (STUB)

| API | Category | Behavior |
|-----|----------|----------|
| `navigator.permissions.query` | STUB | Return {state: 'prompt'} for all |
| `Notification` | STUB | permission = 'default', constructor no-ops |
| `navigator.clipboard` | COMPAT | In-memory read/write |
| `navigator.geolocation` | STUB | Error callback: position unavailable |
| `navigator.mediaDevices` | EXCLUDED | Throws NotAllowedError |

---

## Fingerprint Coherence Baseline

NOT stealth. NOT anti-detection guarantee. A coherence baseline so casual checks don't immediately flag us:

- `navigator.webdriver` = false
- `navigator.userAgent` matches rquest TLS (Chrome 136)
- `navigator.plugins` = empty PluginArray (Chrome removed NPAPI)
- `window.chrome` exists with `{runtime: {}}` shape
- No `__puppeteer`, `__selenium`, `__webdriver`, `__nightmare` globals
- `navigator.permissions.query({name:'notifications'})` returns proper Promise

Sites with real fingerprinting (Canvas, WebGL, AudioContext, font enumeration) WILL detect us. This is a documented limitation, not a bug.

---

## Event Default Action Model

When the shim fires an event, does it also execute the browser's default action?

| Event | Default action | Shim behavior |
|-------|---------------|---------------|
| `click` on `<a>` | Navigate | YES — check href, navigate if not prevented |
| `click` on `<button type=submit>` | Submit form | YES — find form, submit if not prevented |
| `submit` on `<form>` | Send HTTP request | YES — serialize and navigate |
| `keydown` Enter on `<input>` in form | Submit form | COMPAT — simplified: submit if form has submit button. Real spec is complex (implicit submission rules). Marked as compat, not full semantic. |
| `keydown` Enter on `<textarea>` | Insert newline | NO — just the event |
| `keydown` Tab | Focus next element | YES — move activeElement |
| `keydown` Escape | Close modal/dialog | Depends — dispatch event, check if dialog closes |

Rule: shim fires the event FIRST. If `event.defaultPrevented` after dispatch → skip default action. Otherwise → execute default action.

---

## Cross-Origin Frame Policy

- Same-origin iframes: full access (eval inside frame's document)
- Cross-origin iframes: BLOCKED. Return `CrossOriginFrame` error.
- Frame discovery: `list_frames()` returns all iframes with src, name, same-origin flag
- Frame switching: `frame(selector_or_name)` works for same-origin only

---

## Phases

### Phase 1: Navigation + State + Cookies (SEMANTIC)
- form.submit() with full semantics (action, method, enctype, submitter, target)
- location.* (href, assign, replace, reload, hash)
- <a>.click() with defaultPrevented, modifiers, target, download, hash
- document.cookie get/set with correct attribute parsing
- history pushState/replaceState/back/forward with navigation model
- Base URL resolution
- Navigation queue → engine → HTTP → DOM reload

**Gate assertions**:
- DuckDuckGo: type in input → submit form → new page loads with results → extract >5 result titles
- HN login: fill user+pass → submit → response page contains "Bad login" or redirect
- SPA: pushState changes URL without HTTP request, popstate fires on back()

### Phase 2: Document State + Timing + Observers (COMPAT)
- navigator.* (userAgent, webdriver, language, etc.)
- document.readyState lifecycle
- IntersectionObserver, ResizeObserver (compat stubs)
- matchMedia, getComputedStyle (compat stubs)
- requestAnimationFrame, requestIdleCallback
- performance.now(), crypto.*
- Focus management (activeElement tracking)

**Gate assertions**:
- Top 8 sites from benchmark: zero new JS exceptions (compare error count before/after)
- React site with IntersectionObserver: specific lazy-loaded element has content (not just "loads"). Also measure: total DOM node count (detect over-trigger if >10x expected), memory baseline before/after
- MutationObserver test suite: document exact support level

### Phase 3: Network Compat + Remaining (COMPAT/STUB)
- AbortController (semantic — needed for React)
- XMLHttpRequest (compat, async only)
- Blob/File/FileReader basics
- Permissions stubs
- Fingerprint coherence baseline

**Gate assertions**:
- Site using fetch+AbortController: no crash
- Site using XHR: request completes (async)
- `navigator.webdriver === false` verified in eval
