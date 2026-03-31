# PDR: Real Hydration — Minimum Runtime for AI Browser SPA Execution

## Status
- ChatGPT: 323 DOM nodes, React mounted ✅
- 8/10 top sites work ✅
- Mercadona: script decompression bug (separate fix)
- Goal: 95%+ of SPAs hydrate enough for AI to read and interact

## Grok Review Summary (combined)

### Already crossed the hardest line
ChatGPT mount (27→323 nodes) proves the core works. What remains is targeted shims + bug fixes.

### Minimum API surface: ~40 APIs (not 2000+)
Same surface as happy-dom + Vitest for running Next.js apps in CI.

---

## Tier A: MUST be real (hydration fails without these)

| API | Why | Status |
|-----|-----|--------|
| Full DOM (Element, Node, Text, DocumentFragment) | Fiber reconciliation | ✅ linkedom |
| innerHTML / outerHTML | SSR → client tree | ✅ linkedom |
| createElement / createTextNode / createComment | React creates nodes | ✅ linkedom |
| appendChild / insertBefore / removeChild / replaceChild | DOM mutations | ✅ linkedom |
| setAttribute / getAttribute / removeAttribute | Props | ✅ linkedom |
| querySelector / querySelectorAll / getElementById | Element lookup | ✅ linkedom |
| parentNode / childNodes / nextSibling / previousSibling | Tree traversal | ✅ linkedom |
| classList (add/remove/contains/toggle) | Class manipulation | ✅ linkedom |
| textContent / value / checked / selected | Content access | ✅ linkedom |
| addEventListener / removeEventListener / dispatchEvent | Event system | ✅ linkedom |
| MutationObserver | React suspense/portal tracking | ✅ linkedom |
| Event loop drainage (micro + macro) | Mount scheduling | ✅ 50+20 pump |
| DOMContentLoaded / load events | Mount triggers | ✅ dispatched |
| fetch / XMLHttpRequest | Data fetching | ✅ op_fetch |
| document.cookie | Auth | ✅ shim → SQLite |
| window.location + history.pushState/replaceState | SPA routing | ✅ browser_shim |

## Tier B: Must be real OR smart stub (95% of SPAs need)

| API | Why | Status | Action |
|-----|-----|--------|--------|
| IntersectionObserver | Lazy loading, infinite scroll | ✅ stub (fires instantly) | — |
| ResizeObserver | Layout, modals, responsive | ✅ stub (1920x1080) | — |
| requestIdleCallback | React scheduler | ✅ stub (setTimeout 0) | — |
| requestAnimationFrame | Layout effects | ✅ stub (setTimeout 16) | — |
| performance.now() | Timing | ✅ real V8 | — |
| navigator.userAgent/platform/language | UA sniffing | ✅ Chrome 136 | — |
| crypto.getRandomValues | UUID gen | ✅ real V8 | — |
| crypto.subtle.digest | Integrity checks | ✅ stub | — |
| URL / URLSearchParams | URL parsing | ✅ V8 built-in | — |
| FormData | Form serialization | ✅ linkedom | — |
| AbortController / AbortSignal | Fetch cancellation (React) | ⚠️ CHECK | Add if missing |
| CustomEvent / InputEvent / KeyboardEvent / MouseEvent | Event dispatch | ⚠️ CHECK | Ensure constructors work |
| MessageChannel | React scheduler internals | ❌ MISSING | **ADD** |
| DOMParser | HTML string parsing | ⚠️ CHECK | Add if missing |
| document.createRange | React text insertion | ✅ stub added | — |
| window.getSelection | React selection | ✅ stub added | — |
| focus() / blur() | Focus model | ✅ shim | — |

## Tier C: Optional / site-dependent

| API | When needed | Action |
|-----|-------------|--------|
| localStorage / sessionStorage | Auth, preferences | ✅ already have |
| matchMedia | Responsive JS | ✅ stub (desktop) |
| getComputedStyle | Visibility checks | ✅ stub (defaults) |
| popstate event | SPA back/forward | ✅ history shim |
| Blob / File / FileReader | Upload forms | ✅ basic impl |
| XMLSerializer | Rare | Skip unless needed |
| PointerEvent | Modern click handlers | Add if sites break |

## Tier D: Safe to ignore

- navigator.serviceWorker (stub exists, never real)
- caches (Cache API)
- Canvas / WebGL / WebAudio
- document.fonts
- WebSocket (stub or exclude)
- screen orientation
- Notification API
- Geolocation
- Media devices
- Full navigation timing API
- Service Workers
- Web Workers (unless critical site needs)

---

## Runtime Semantics (ranked by hydration importance)

### Rank 1: Critical for mount
1. **Microtask drainage** — Promise.then, queueMicrotask run before macrotasks ✅
2. **Module evaluation completion** — dynamic import() chains fully resolve ✅
3. **Script ordering** — blocking → defer → async, DOMContentLoaded between ✅
4. **DOM mutation timing** — mutations are synchronous, observer callbacks are microtasks ✅

### Rank 2: Critical for interaction
5. **Event propagation** — capture → target → bubble, stopPropagation works
6. **Default actions** — click on `<a>` navigates, submit on form submits (unless prevented)
7. **Focus model** — focus/blur/focusin/focusout fire correctly
8. **Fetch completion** — response.text(), response.json() resolve properly ✅

### Rank 3: Important for correctness
9. **Timer semantics** — setTimeout(0) = macrotask, nested clamping ✅
10. **Promise rejection surfacing** — unhandled rejections don't silently swallow errors
11. **Top-level await** — modules with TLA complete evaluation
12. **Client-side navigation** — pushState updates URL, popstate fires on back

### Rank 4: Nice to have
13. **Streaming fetch** — response.body as ReadableStream ✅
14. **Text decoding** — TextDecoder/TextEncoder ✅
15. **Idle detection** — when to stop pumping event loop

---

## Interaction Semantics for Real App Logic

### Typing must update framework state
```
element.focus()                          // React attaches onFocus
element.dispatchEvent(new Event('focus'))
// Per character:
element.dispatchEvent(new KeyboardEvent('keydown', {key, code, bubbles:true}))
element.value += char                    // OR use InputEvent
element.dispatchEvent(new InputEvent('input', {data:char, inputType:'insertText', bubbles:true}))
element.dispatchEvent(new KeyboardEvent('keyup', {key, code, bubbles:true}))
// After all chars:
element.dispatchEvent(new Event('change', {bubbles:true}))
```
**CRITICAL**: `element.value = text` alone is INSUFFICIENT for React. React listens to `input` event, not value change. Must dispatch InputEvent.

### Clicking must trigger handlers
```
element.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true}))
element.dispatchEvent(new MouseEvent('mousedown', {bubbles:true}))
element.dispatchEvent(new PointerEvent('pointerup', {bubbles:true}))
element.dispatchEvent(new MouseEvent('mouseup', {bubbles:true}))
element.dispatchEvent(new MouseEvent('click', {bubbles:true}))
```
React 17+ uses event delegation on root — events MUST bubble.

### Enter submits
```
element.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', code:'Enter', bubbles:true}))
// If in form with submit button: form.requestSubmit() or form.submit()
element.dispatchEvent(new KeyboardEvent('keyup', {key:'Enter', code:'Enter', bubbles:true}))
```

### Checkbox/select/file
- Checkbox: set `.checked = true` + dispatch `change` event
- Select: set `.value` + dispatch `change` event
- File: set `.files` (synthetic FileList) + dispatch `change` event

---

## Implementation Roadmap

### Phase 0: Instrument (DONE mostly)
- NEORENDER_TRACE=1 traces ✅
- Module resolution traces ✅
- Script execution traces ✅
- Event loop pump count ✅
- **ADD**: unhandled rejection logging
- **ADD**: missing API access logging (Proxy trap on window for undefined properties)

### Phase 1: Highest-leverage runtime fixes
- **MessageChannel** — React scheduler uses this for concurrent features
- **AbortController verification** — confirm it works, fix if not
- **Event constructor verification** — InputEvent, PointerEvent, CustomEvent constructors
- **Unhandled promise rejection logging** — surface silent failures
- Gate: ChatGPT WOM nodes stay ≥300, no new errors

### Phase 2: Highest-leverage DOM/API additions
- **DOMParser** — `new DOMParser().parseFromString(html, 'text/html')`
- **PointerEvent** — modern click handlers use this
- **InputEvent with data** — React controlled inputs need this
- **Event bubbling verification** — confirm events bubble to document root (React delegation)
- Gate: Vercel.com errors drop from 15 to <5

### Phase 3: Interaction correctness
- Update LiveDom click to use full pointer+mouse sequence
- Update LiveDom type_text to dispatch InputEvent per character
- Update LiveDom submit to use requestSubmit when available
- Gate: form fill+submit works on httpbin, DuckDuckGo

### Phase 4: Framework edge cases (ONLY if still needed)
- Only after evidence from traces
- No pre-emptive patches
- Gate: one additional SPA site works (nuxt.com, svelte.dev, etc.)

---

## Hard Exclusions (do NOT build)

- Full layout engine
- CSSOM completeness (getComputedStyle returns stubs, that's fine)
- Visual rendering / screenshots
- WebGL / Canvas parity
- Accessibility tree parity
- Service Workers (real)
- Full Navigation Timing API
- Anti-bot / stealth work (coherent fingerprint is enough)
- Framework-specific hacks before runtime evidence
- Web Workers (unless a critical site needs them)

---

## Mercadona Bug (separate)

Not a hydration issue. The 792KB script downloads OK but V8 gets `SyntaxError: Unexpected end of input`. Probable cause: response body decompression issue in rquest or encoding mismatch. Fix: verify Content-Encoding handling in op_fetch, ensure gzip/brotli/zstd fully decoded before passing to V8.

---

## Gate

### Phase 1 gate
- MessageChannel exists and works
- AbortController works
- Event constructors work (InputEvent, PointerEvent, CustomEvent)
- ChatGPT: ≥300 DOM nodes maintained
- No new errors on existing 8/10 sites

### Phase 2 gate
- DOMParser works
- Event bubbling reaches document root
- Vercel.com: errors < 5 (currently 15)

### Phase 3 gate
- LiveDom type dispatches InputEvent (React state updates)
- LiveDom click dispatches PointerEvent sequence
- httpbin form fill+submit works end-to-end

### Final validation
- ChatGPT: interactive (textarea fillable, send button clickable via events)
- 3+ Next.js/Vue/Svelte sites load with >50% of expected content
- All 340+ existing tests pass
