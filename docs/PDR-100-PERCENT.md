# PDR: NeoRender V2 — 100% Browser Completion

## Current State (22 March 2026)

| Capability | Status | Notes |
|---|---|---|
| SSR render (HN, Wikipedia, GitHub) | DONE | 10/10 sites, WOM extraction working |
| SPA render (Mercadona, React/Vue) | DONE | React 18 hydration, 329/354 fibers on ChatGPT |
| Form interaction (type, click) | PARTIAL | fill_form works for React, basic form.submit() |
| ChatGPT PONG | BLOCKED | Send works, SSE response not captured |
| Cookie auto-injection | PARTIAL | JS-side `__getCookiesForUrl` exists but `__neorender_cookies` not populated from SQLite store |
| Streaming responses (SSE) | MISSING | No EventSource, no streaming fetch body |
| Multi-page sessions | PARTIAL | History API works, real re-navigation limited |
| Web Components | FREE | happy-dom has customElements built-in |
| WebSocket | STUB | happy-dom has WebSocket class but no real I/O |

---

## Phase 1: Critical Fixes (Unblock PONG)

### 1.1 — SSE / Streaming Response Support

**Problem:** ChatGPT sends messages via streaming POST (Content-Type: `text/event-stream`). The current `op_fetch` reads the entire response body synchronously via `spawn_blocking`, returning one JSON blob. There's no way for JS to consume a response body incrementally. No `EventSource` implementation exists.

**Proposed fix — two parts:**

#### Part A: Streaming fetch body

New async op `op_fetch_stream` that returns chunks progressively.

**Files to change:**
- `crates/neo-runtime/src/ops.rs` — add `op_fetch_stream_start(url, method, body, headers) -> stream_id` (async, opens connection, returns ID + first chunk + headers)
- `crates/neo-runtime/src/ops.rs` — add `op_fetch_stream_read(stream_id) -> chunk | null` (async, reads next chunk)
- `crates/neo-runtime/src/ops.rs` — add `op_fetch_stream_close(stream_id)` (sync, drops connection)
- `crates/neo-runtime/src/v8.rs` — register new ops in the extension
- New `crates/neo-runtime/src/stream_store.rs` — holds open rquest response bodies keyed by stream_id, stored in OpState
- `js/bootstrap.js` — modify `NeoResponse.body` getter to return a real streaming `ReadableStream` when response has `_streamId`
- `js/bootstrap.js` — modify `fetch()` to detect streaming content-types (`text/event-stream`, `application/x-ndjson`, `text/plain` with Transfer-Encoding chunked) and use streaming path

**Approach in Rust:**
```
// stream_store.rs
pub struct StreamStore {
    streams: HashMap<u32, tokio::sync::mpsc::Receiver<Vec<u8>>>,
    next_id: AtomicU32,
}

// op_fetch_stream_start: spawn a tokio task that reads rquest::Response
// chunk by chunk into an mpsc channel. Return stream_id + status + headers.
// op_fetch_stream_read: recv().await from the channel → return chunk or null on close.
```

**JS side:**
```js
// In fetch(), detect streaming response:
if (isStreamingContentType(result.contentType)) {
    const streamId = result.streamId;
    resp._streamId = streamId;
    // Override .body getter to return ReadableStream backed by op_fetch_stream_read
}
```

#### Part B: EventSource polyfill

**File:** `js/bootstrap.js` — add `EventSource` class after fetch section.

```js
class EventSource {
    constructor(url, opts) {
        this.url = url;
        this.readyState = 0; // CONNECTING
        this.CONNECTING = 0; this.OPEN = 1; this.CLOSED = 2;
        this._listeners = { message: [], open: [], error: [] };
        this._start();
    }
    _start() {
        // Use streaming fetch internally
        fetch(this.url, { headers: { 'Accept': 'text/event-stream' }})
            .then(resp => {
                this.readyState = 1;
                this._fireEvent('open', {});
                return this._consumeStream(resp.body);
            })
            .catch(err => { this.readyState = 2; this._fireEvent('error', err); });
    }
    _consumeStream(body) {
        const reader = body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';
        const pump = () => reader.read().then(({ done, value }) => {
            if (done || this.readyState === 2) return;
            buffer += decoder.decode(value, { stream: true });
            // Parse SSE: split on \n\n, extract event/data/id
            const events = buffer.split('\n\n');
            buffer = events.pop(); // incomplete event stays in buffer
            for (const raw of events) {
                if (!raw.trim()) continue;
                const msg = this._parseSSE(raw);
                if (msg) this._fireEvent(msg.event || 'message', msg);
            }
            pump();
        });
        pump();
    }
    // ... parseSSE, addEventListener, close, etc.
}
```

**Complexity:** HIGH (streaming fetch requires new Rust ops + new JS response path)
**Dependencies:** None
**Test criteria:**
- `fetch('https://httpbin.org/stream/5').then(r => r.body.getReader())` returns chunks
- ChatGPT conversation POST returns streamed SSE events
- `new EventSource(url)` receives `onmessage` callbacks

**Estimated effort:** 2 sessions

---

### 1.2 — Cookie Auto-Injection from SQLite Store

**Problem:** `NeoSession` has `cookie_store: Option<Box<dyn CookieStore>>` with a working `SqliteCookieStore`. The JS fetch wrapper uses `__neorender_cookies` (a JS object). But nobody populates `__neorender_cookies` from the Rust cookie store. Cookies imported via Chrome importer sit in SQLite but never reach HTTP requests made by page JS.

Two gaps:
1. **JS→Rust direction (op_fetch):** JS `fetch()` reads cookies from `__neorender_cookies` and adds a `Cookie` header. But `__neorender_cookies` is empty because no one loads cookies from SQLite into it.
2. **Rust response→Store direction:** `Set-Cookie` response headers from `op_fetch` are not stored back into the cookie jar.

**Proposed fix:**

**Files to change:**
- `crates/neo-runtime/src/ops.rs` — modify `op_fetch` to:
  1. Before sending: read cookies from `CookieStore.get_for_request(url, top_level_url, is_top_level)` and merge into request headers (if no `Cookie` header already set by JS)
  2. After receiving: parse `Set-Cookie` response headers and call `CookieStore.store_set_cookie(url, header)` for each
- `crates/neo-runtime/src/ops.rs` — add `SharedCookieStore` wrapper struct (like `SharedHttpClient`)
- `crates/neo-runtime/src/v8.rs` — inject `SharedCookieStore` into OpState during runtime creation
- `crates/neo-engine/src/session/pipeline.rs` or `session/mod.rs` — pass cookie_store Arc to the V8 runtime builder
- `js/bootstrap.js` — add `op_cookie_get_for_url(url)` call in `__getCookiesForUrl` as fallback when `__neorender_cookies` is empty
- Optionally: populate `__neorender_cookies` from Rust at bootstrap time via a sync op `op_cookie_export_for_domain(domain) -> JSON`

**Approach — Rust-side auto-inject (preferred):**
```rust
// In op_fetch, before building HttpRequest:
let cookie_header = {
    let s = state.borrow();
    if let Some(cs) = s.try_borrow::<SharedCookieStore>() {
        let h = cs.0.get_for_request(&url, None, true);
        if !h.is_empty() { Some(h) } else { None }
    } else { None }
};
// Merge into headers if not already present
if let Some(ch) = cookie_header {
    headers.entry("Cookie".to_string()).or_insert(ch);
}

// After response:
if let Ok(ref resp) = result {
    if let Some(set_cookies) = resp.headers.get("set-cookie") {
        let s = state.borrow();
        if let Some(cs) = s.try_borrow::<SharedCookieStore>() {
            for sc in set_cookies.split('\n') { // multi-value
                cs.0.store_set_cookie(&url, sc);
            }
        }
    }
}
```

**Complexity:** MEDIUM
**Dependencies:** None
**Test criteria:**
- Import Chrome cookies → navigate to ChatGPT → subsequent fetch() calls include auth cookies
- `Set-Cookie` from server responses persists to SQLite and is sent on next request
- Existing `__neorender_cookies` JS-side injection still works as override

**Estimated effort:** 1 session

---

### 1.3 — happy-dom ReadableStream Fix

**Problem:** happy-dom's `ReadableStream.getReader().read()` hangs (never resolves). This blocks turbo-stream decode (React Router 7 SSR) and will block SSE consumption in Phase 1.1.

**Current workaround:** Turbo-stream interceptor in `browser_shim.js` patches `streamController.enqueue/close` to accumulate chunks and decode synchronously.

**Proposed fix:**

**File:** `js/bootstrap.js` — the polyfill ReadableStream (section 7b, line ~951) already exists with `_neo_polyfill` flag. The issue is that happy-dom installs its own ReadableStream AFTER our bootstrap but BEFORE page scripts.

Strategy: ensure our working ReadableStream polyfill always wins.

```js
// At the END of bootstrap.js, after happy-dom globals are installed:
// Force our ReadableStream over happy-dom's broken one
if (globalThis.ReadableStream && !globalThis.ReadableStream.prototype._neo_polyfill) {
    // happy-dom replaced ours — swap back
    globalThis.ReadableStream = NeoReadableStream; // saved reference
}
```

Actually, the real fix is simpler: make `NeoResponse.body` always use our polyfill ReadableStream, not happy-dom's.

**File:** `js/bootstrap.js` — in `NeoResponse.body` getter (line ~253), ensure we use the saved `NeoReadableStream` class explicitly.

**Complexity:** LOW
**Dependencies:** None (but enables 1.1 streaming)
**Test criteria:**
- `new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1])); c.close(); } }).getReader().read()` resolves with `{done: false, value: Uint8Array}`
- turbo-stream decode works without the streamController interceptor hack

**Estimated effort:** 0.5 sessions

---

## Phase 2: Interaction Reliability

### 2.1 — React 16 Event Delegation Fix

**Problem:** React 16 (Mercadona) uses `document.addEventListener` for event delegation at the document level. In happy-dom, `listenToAllSupportedEvents` doesn't install properly because happy-dom's event dispatch doesn't bubble through the exact same path React 16 expects. React 18 (ChatGPT) uses root-level delegation which works with our `dispatchEvent` monkeypatch.

**Current state:** `browser_shim.js` has a `dispatchEvent` monkeypatch that wraps events with `_neoReactSynth: true`. This works for React 18 but React 16's `_getClosestInstanceFromNode` lookup fails.

**Proposed fix:**

**Files to change:**
- `crates/neo-runtime/src/modules.rs` — add source transform for React 16: detect `listenToAllSupportedEvents` or `listenToNativeEvent` pattern in react-dom bundle
- `js/browser_shim.js` — add React 16 event bridge:
  ```js
  // After React 16 renders, find __reactInternalInstance on DOM nodes
  // Wrap our click/input events to include fiber reference:
  // el[__reactInternalInstance$xxx] must exist for React 16 SyntheticEvent
  ```
- `crates/neo-engine/src/live_dom.rs` — in `fill_form`, detect React version from DOM (`__reactFiber` = 18, `__reactInternalInstance` = 16) and adjust event dispatch

**Alternative approach (simpler):** Source-transform React 16's `ensureListeningTo` to be a no-op, and directly call `_dispatchEventWhenListening` on the element with the correct event.

**Complexity:** MEDIUM
**Dependencies:** None
**Test criteria:**
- Mercadona postal code form: `fill_form` + click "Ver tienda" triggers navigation
- React 16 `onChange` handlers fire on `fill_form` input events

**Estimated effort:** 1.5 sessions

---

### 2.2 — SPA Client-Side Routing

**Problem:** `pushState`/`replaceState` update location but don't trigger re-render of route-matched components. React Router, Vue Router, and Svelte all listen for `popstate` to re-render, but programmatic `pushState` doesn't fire `popstate` (per spec).

**Current state:** `browser_shim.js` tracks history entries and dispatches `popstate` on `back()`/`forward()`. But SPA link clicks that call `pushState` don't trigger the route change in the framework.

**Proposed fix:**

**Files to change:**
- `js/browser_shim.js` — in `pushState` handler, after updating location, dispatch a custom `__neo_route_change` event AND attempt to call the framework's navigation function:
  ```js
  // After pushState:
  // 1. Dispatch popstate (some routers listen for it even on push)
  try { globalThis.dispatchEvent(new PopStateEvent('popstate', { state: state })); } catch {}
  // 2. For React Router: __reactRouterContext may have router.navigate()
  // 3. For Vue Router: window.__vue_router__?.push(url)
  ```
- `crates/neo-engine/src/session/pipeline.rs` — add `navigate_spa(url)` method that:
  1. Calls `history.pushState` in V8
  2. Dispatches `popstate`
  3. Runs settle loop (wait for re-render)
  4. Re-extracts WOM
- `crates/neo-mcp/src/tools/browse.rs` — expose `navigate_spa` as MCP option (or auto-detect when URL is same-origin)

**Complexity:** MEDIUM
**Dependencies:** None
**Test criteria:**
- Mercadona: navigate from home → categories page via SPA link → WOM shows new content
- React Router app: `navigate("/about")` → about page renders in WOM

**Estimated effort:** 1 session

---

### 2.3 — Rich Text Editor Bridge (Generalized)

**Problem:** ChatGPT uses ProseMirror. Other apps use Tiptap (ProseMirror wrapper), Lexical (Meta), Slate, CodeMirror, Quill. Each has its own internal state model that ignores DOM `value` changes.

**Current state:** ProseMirror captured via source transform on `this.domObserver=` pattern. `tr.insertText("text") + view.dispatch(tr)` works for ChatGPT specifically.

**Proposed fix — generic editor detection + bridge:**

**Files to change:**
- `js/browser_shim.js` — add `__neo_editor_registry` that auto-detects editor instances:
  ```js
  const __neo_editors = {
      prosemirror: null, // ProseMirror EditorView
      lexical: null,     // Lexical editor instance
      slate: null,       // Slate editor
      codemirror: null,  // CodeMirror EditorView
  };
  // Detection: watch for known globals/patterns
  // ProseMirror: already captured via source transform
  // Lexical: window.__lexicalEditor or document.querySelector('[data-lexical-editor]').__lexical
  // Slate: ReactEditor from slate-react module
  ```
- `crates/neo-runtime/src/modules.rs` — add source transforms for:
  - Lexical: capture `createEditor()` result
  - Slate: capture `withReact(editor)` result
  - CodeMirror: capture `new EditorView(...)` result
- `crates/neo-engine/src/live_dom.rs` — in `fill_form` / new `type_rich_text()`, dispatch through the detected editor:
  ```js
  function __neo_type_rich(text) {
      if (__neo_editors.prosemirror) { /* tr.insertText + dispatch */ }
      else if (__neo_editors.lexical) { /* editor.update(() => { insertText(text) }) */ }
      else if (__neo_editors.slate) { /* Transforms.insertText(editor, text) */ }
      else { /* fallback: DOM input events */ }
  }
  ```

**Complexity:** HIGH (each editor is different, needs testing per-site)
**Dependencies:** 1.1 (for ChatGPT response verification)
**Test criteria:**
- ChatGPT: type message → ProseMirror state updates → send button enables
- A Lexical-based app: type text → editor state updates
- Fallback: contenteditable without framework still works via InputEvent

**Estimated effort:** 2 sessions

---

### 2.4 — Form Submit Handling Improvements

**Problem:** `form.submit()` captures form data and sends `op_navigation_request`, but:
1. `fetch`-based form submissions (SPA) aren't intercepted
2. `<button type="submit">` click doesn't always trigger form submit event
3. File upload fields are ignored
4. Multipart/form-data encoding missing

**Proposed fix:**

**Files to change:**
- `js/browser_shim.js` — improve `__neoFormSubmit`:
  - Add `enctype` detection (`application/x-www-form-urlencoded` vs `multipart/form-data`)
  - Dispatch `submit` event before navigation request (so SPA interceptors can preventDefault)
  - Handle `<button formaction="">` override
- `crates/neo-interact/src/forms.rs` — add `submit_form(selector)` that:
  1. Clicks the submit button (dispatching click event)
  2. Waits for form submit event
  3. If not prevented → send navigation request
  4. If prevented → wait for SPA handler to finish (fetch + re-render)
- `js/browser_shim.js` — intercept `form.addEventListener('submit', ...)` to track SPA form handlers

**Complexity:** MEDIUM
**Dependencies:** None
**Test criteria:**
- HTML form POST → navigation request with form data
- React form with `onSubmit={e => { e.preventDefault(); fetch(...) }}` → fetch fires, no navigation
- Form with file input → upload data included (as filename reference)

**Estimated effort:** 1 session

---

## Phase 3: Browser Compat

### 3.1 — Web Components (customElements)

**Problem:** happy-dom has `customElements.define()` built-in, but Shadow DOM attachment and slot distribution may not work for complex component libraries (Shoelace, Lit, etc.).

**Current state:** happy-dom exports `customElements` via Window. No custom testing done.

**Proposed fix:**

**Files to change:**
- `js/bootstrap.js` — ensure `customElements` is exported to global scope (may already be via happy-dom Window)
- `js/bootstrap.js` — verify `element.attachShadow({ mode: 'open' })` works, add polyfill if not
- Test with a Lit element to validate the full lifecycle (define → construct → connectedCallback → render)

**Complexity:** LOW (happy-dom does most of the work)
**Dependencies:** None
**Test criteria:**
- `customElements.define('my-el', class extends HTMLElement { ... })` registers
- `document.createElement('my-el')` instantiates with correct lifecycle
- Lit SSR example renders correctly

**Estimated effort:** 0.5 sessions

---

### 3.2 — WebSocket Support

**Problem:** happy-dom has a WebSocket class but it connects to nothing in headless mode. Some SPAs use WebSocket for real-time updates (chat apps, dashboards).

**Proposed fix:**

**Files to change:**
- `crates/neo-runtime/src/ops.rs` — add ops:
  - `op_ws_connect(url, protocols) -> ws_id` (async, opens WebSocket via tokio-tungstenite)
  - `op_ws_send(ws_id, data)` (sync, queues message)
  - `op_ws_read(ws_id) -> message | null` (async, receives next message)
  - `op_ws_close(ws_id)` (sync)
- `crates/neo-runtime/src/ws_store.rs` — holds open WebSocket connections keyed by ID
- `js/bootstrap.js` — replace happy-dom's WebSocket with one that routes through Rust ops:
  ```js
  class NeoWebSocket {
      constructor(url, protocols) {
          this.url = url;
          this.readyState = 0; // CONNECTING
          this._connect(url, protocols);
      }
      _connect(url, protocols) {
          ops.op_ws_connect(url, JSON.stringify(protocols || []))
              .then(wsId => {
                  this._id = wsId;
                  this.readyState = 1;
                  if (this.onopen) this.onopen(new Event('open'));
                  this._pump();
              })
              .catch(err => { this.readyState = 3; if (this.onerror) this.onerror(err); });
      }
      _pump() { /* loop: op_ws_read → onmessage */ }
      send(data) { ops.op_ws_send(this._id, data); }
      close() { ops.op_ws_close(this._id); this.readyState = 2; }
  }
  ```

**Complexity:** HIGH
**Dependencies:** None (but similar pattern to 1.1 streaming)
**Test criteria:**
- `new WebSocket('wss://echo.websocket.org')` connects and echoes
- `ws.send('hello')` → `ws.onmessage` receives response
- `ws.close()` cleans up

**Estimated effort:** 1.5 sessions

---

### 3.3 — Improved CSS Support

**Problem:** happy-dom has basic CSS parsing but `getComputedStyle()` returns incomplete values. No layout engine, so `offsetWidth`, `getBoundingClientRect()` return zeros. Some SPAs use these for conditional rendering (responsive design, virtual scrolling).

**Proposed fix (pragmatic — no layout engine):**

**Files to change:**
- `js/bootstrap.js` — improve `getComputedStyle` stub:
  - Parse inline `style` attributes and return those values
  - Parse `<style>` tags and match selectors (basic: class, id, tag)
  - For layout properties (`width`, `height`, `display`), return sensible defaults based on element type
- `js/bootstrap.js` — improve `getBoundingClientRect` stub:
  - Return non-zero dimensions based on element type (div=100%, img=width/height attrs, etc.)
  - Track cumulative offsets for basic layout estimation
- `js/bootstrap.js` — `offsetWidth`/`offsetHeight` return values from getBoundingClientRect

**Complexity:** MEDIUM (endless rabbit hole — scope carefully)
**Dependencies:** None
**Test criteria:**
- `getComputedStyle(el).display` returns correct value for block/inline elements
- `el.getBoundingClientRect().width > 0` for visible elements
- Virtual scroll libraries (react-window) don't crash

**Estimated effort:** 1 session (scoped to "don't crash" level, not pixel-perfect)

---

### 3.4 — Performance Optimization

**Problem:** ChatGPT loads 7+ ES modules with heavy dependency trees. Module fetch + compile + evaluate dominates load time.

**Proposed fix:**

**Files to change:**
- `crates/neo-runtime/src/code_cache.rs` — V8 code cache already exists. Verify it's being used for all modules (check `get_source_code_cache_info` implementation)
- `crates/neo-runtime/src/modules.rs` — parallelize module fetches: when a module has N imports, fetch all N simultaneously (current: sequential per import discovery)
- `crates/neo-engine/src/session/prefetch.rs` — extend prefetch to cover dynamic imports discovered during execution
- `js/bootstrap.js` — lazy-init heavy polyfills (only create ReadableStream polyfill when first accessed)

**Complexity:** MEDIUM
**Dependencies:** None
**Test criteria:**
- ChatGPT page load < 5 seconds (currently ~8-10s)
- Second visit to same page uses code cache (< 3s)
- Module fetch parallelism visible in trace logs

**Estimated effort:** 1 session

---

## Summary & Effort Estimate

| # | Item | Phase | Complexity | Sessions | Blocked by |
|---|---|---|---|---|---|
| 1.1 | SSE / Streaming fetch | 1 | HIGH | 2.0 | — |
| 1.2 | Cookie auto-injection | 1 | MEDIUM | 1.0 | — |
| 1.3 | ReadableStream fix | 1 | LOW | 0.5 | — |
| 2.1 | React 16 events | 2 | MEDIUM | 1.5 | — |
| 2.2 | SPA routing | 2 | MEDIUM | 1.0 | — |
| 2.3 | Rich text editors | 2 | HIGH | 2.0 | 1.1 |
| 2.4 | Form submit | 2 | MEDIUM | 1.0 | — |
| 3.1 | Web Components | 3 | LOW | 0.5 | — |
| 3.2 | WebSocket | 3 | HIGH | 1.5 | — |
| 3.3 | CSS support | 3 | MEDIUM | 1.0 | — |
| 3.4 | Performance | 3 | MEDIUM | 1.0 | — |
| | **TOTAL** | | | **13.0** | |

### Critical path for ChatGPT PONG:

```
1.3 ReadableStream fix (0.5 sessions)
  └→ 1.2 Cookie auto-injection (1.0 session)
      └→ 1.1 SSE streaming (2.0 sessions)
          └→ PONG WORKS (3.5 sessions total)
```

### Recommended execution order:

1. **1.3** ReadableStream fix — quick win, unblocks streaming
2. **1.2** Cookie auto-injection — essential for authenticated sites
3. **1.1** SSE streaming — the big one, unlocks PONG
4. **2.2** SPA routing — high impact for general browsing
5. **2.4** Form submit improvements — interaction reliability
6. **2.1** React 16 events — Mercadona and legacy React sites
7. **2.3** Rich text editors — generalize beyond ProseMirror
8. **3.1** Web Components — quick if happy-dom works
9. **3.4** Performance — cache and parallelism
10. **3.2** WebSocket — niche but needed for real-time apps
11. **3.3** CSS support — endless rabbit hole, do last

### Key architectural decisions:

1. **Streaming at Rust level, not JS level.** The `op_fetch_stream_*` ops hold the connection open in Rust (tokio) and feed chunks to JS via async ops. This avoids happy-dom's broken ReadableStream entirely.

2. **Cookie injection at Rust level.** Modify `op_fetch` directly to read/write cookies from `SharedCookieStore` in OpState. This is more reliable than populating a JS object and covers all request types.

3. **Editor detection via source transforms.** Same pattern that works for ProseMirror — intercept construction in module source code, capture instance to global registry. No runtime scanning needed.

4. **No layout engine.** Return plausible non-zero values for `getBoundingClientRect` based on element type. Full CSS layout (Servo, Taffy) would add 50K+ lines of code for marginal benefit in AI browser context.
