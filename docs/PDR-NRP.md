# PDR: NeoRender Protocol (NRP)

> Protocol Design Record — v1.0 — 2026-03-22
> Reverse-engineering CDP for AI browser interaction.
> No Chrome. No external browser. Our engine, our protocol.

---

## 1. CDP Architecture Analysis

### 1.1 How CDP Works

Chrome DevTools Protocol is JSON-RPC 2.0 over WebSocket. Messages:

```
Request:  { "id": 1, "method": "Page.navigate", "params": { "url": "..." } }
Response: { "id": 1, "result": { "frameId": "...", "loaderId": "..." } }
Event:    { "method": "Page.loadEventFired", "params": { "timestamp": 1234 } }
```

Three message types:
- **Commands** (client -> server): have `id` + `method` + `params`
- **Responses** (server -> client): have `id` + `result` (or `error`)
- **Events** (server -> client): have `method` + `params`, no `id` — async notifications

Domains are enabled/disabled per-session. Events only fire after `Domain.enable`.

### 1.2 Complete CDP Domain Map

48 domains total. Classified by what NRP does with each:

#### KEEP — Port to NRP (core functionality)

| CDP Domain | Purpose | Key Commands | Key Events |
|-----------|---------|-------------|------------|
| **Page** | Navigation lifecycle | `navigate`, `reload`, `getNavigationHistory`, `stopLoading`, `setLifecycleEventsEnabled` | `loadEventFired`, `domContentEventFired`, `frameNavigated`, `lifecycleEvent`, `javascriptDialogOpening` |
| **DOM** | Document tree R/W | `getDocument`, `querySelector`, `querySelectorAll`, `getAttributes`, `setAttributeValue`, `getOuterHTML`, `setOuterHTML`, `removeNode`, `requestChildNodes` | `documentUpdated`, `childNodeInserted`, `childNodeRemoved`, `attributeModified`, `characterDataModified` |
| **Runtime** | JS execution | `evaluate`, `callFunctionOn`, `getProperties`, `awaitPromise` | `consoleAPICalled`, `exceptionThrown`, `executionContextCreated` |
| **Network** | HTTP traffic observation | `enable`, `getResponseBody`, `setCookie`, `getCookies`, `setExtraHTTPHeaders` | `requestWillBeSent`, `responseReceived`, `loadingFinished`, `loadingFailed` |
| **Input** | User simulation | `dispatchMouseEvent`, `dispatchKeyEvent`, `insertText`, `dispatchTouchEvent` | — |
| **Fetch** | Request interception | `enable`, `continueRequest`, `fulfillRequest`, `failRequest` | `requestPaused` |
| **Storage** | Cookies, storage | `getCookies`, `setCookies`, `clearCookies`, `clearDataForOrigin` | — |
| **DOMStorage** | localStorage/sessionStorage | `getDOMStorageItems`, `setDOMStorageItem`, `removeDOMStorageItem`, `clear` | `domStorageItemAdded`, `domStorageItemUpdated` |
| **Log** | Browser log capture | `enable`, `clear` | `entryAdded` |
| **Target** | Tab/context management | `createTarget`, `attachToTarget`, `closeTarget`, `getTargets` | `targetCreated`, `targetDestroyed` |

#### TRANSFORM — Adapt for AI

| CDP Domain | Purpose | What We Do |
|-----------|---------|-----------|
| **Accessibility** | AX tree from rendered page | Build our own AXtree from DOM (no rendering needed). CDP commands: `getFullAXTree`, `queryAXTree`. Events: `loadComplete`, `nodesUpdated`. We generate equivalent from our DOM engine. |
| **CSS** | Computed styles, stylesheets | Strip most. Keep only: `display:none` detection (visibility), `pointer-events:none` (interactivity). No layout, no computed styles, no style editing. |
| **Emulation** | Device/viewport override | Simplify to user-agent string + viewport dimensions for responsive content. No GPU, no touch emulation, no geolocation. |
| **Console** | Console messages (deprecated) | Merge into our Log domain. CDP deprecated it in favor of Runtime + Log. |
| **Security** | TLS/cert info | Simplify to TLS status + certificate errors only. |

#### STRIP — Remove entirely

| CDP Domain | Purpose | Why Strip |
|-----------|---------|----------|
| **Animation** | CSS/Web animations inspection | Visual only |
| **Audits** | Lighthouse-style audits | DevTools feature |
| **Autofill** | Form autofill data | Browser-specific |
| **BackgroundService** | Service worker background events | Not relevant for AI browsing |
| **Browser** | Browser-level commands (version, crash) | We ARE the browser |
| **CacheStorage** | Cache API inspection | DevTools feature |
| **Cast** | Chromecast control | Hardware-specific |
| **Database** | WebSQL inspection (deprecated) | Dead API |
| **Debugger** | JS breakpoints, stepping | Developer tooling |
| **DeviceAccess** | USB/Bluetooth device access | Hardware-specific |
| **DeviceOrientation** | Gyroscope/accelerometer mock | Sensor emulation |
| **DOMDebugger** | DOM breakpoints | Developer tooling |
| **DOMSnapshot** | DOM tree snapshot (for rendering) | We have our own DOM |
| **EventBreakpoints** | Event listener breakpoints | Developer tooling |
| **Extensions** | Chrome extension management | Browser-specific |
| **HeadlessExperimental** | Headless Chrome APIs | We're already headless |
| **HeapProfiler** | Memory heap inspection | Profiling |
| **IndexedDB** | IndexedDB inspection | Not needed for AI |
| **Inspector** | Inspector lifecycle | DevTools meta |
| **IO** | Stream reading | Internal plumbing |
| **LayerTree** | Compositing layers | Rendering |
| **Media** | Media playback inspection | A/V specific |
| **Memory** | Memory pressure simulation | Profiling |
| **Overlay** | DOM node highlighting overlay | Visual debugging |
| **Performance** | Performance metrics collection | Profiling |
| **PerformanceTimeline** | Performance entries | Profiling |
| **Preload** | Speculation rules, prefetch | Browser optimization |
| **Profiler** | CPU profiling | Developer tooling |
| **PWA** | Progressive Web App inspection | Browser-specific |
| **Schema** | Protocol version info | Meta |
| **ServiceWorker** | SW lifecycle management | Not relevant for AI |
| **SystemInfo** | GPU, CPU, memory info | Hardware info |
| **Tethering** | Port forwarding | DevTools remote debugging |
| **Tracing** | Chrome tracing | Profiling |
| **WebAudio** | Web Audio API inspection | A/V specific |
| **WebAuthn** | WebAuthn/FIDO emulation | Auth-specific |

**Score: 10 KEEP + 5 TRANSFORM + 33 STRIP = 48 total**

We keep ~31% of CDP's surface area. The 33 stripped domains are all about visual rendering, developer debugging, profiling, and hardware — none of which an AI agent needs.

---

## 2. NRP Domain Specification

### 2.1 Protocol Transport

```
JSON-RPC 2.0 over stdio (MCP integration)
JSON-RPC 2.0 over WebSocket (future: remote agents)
```

Message format identical to CDP:
```
Command:  { "id": 1, "method": "Page.navigate", "params": { "url": "..." } }
Response: { "id": 1, "result": { ... } }
Event:    { "method": "Content.domChanged", "params": { ... } }
Error:    { "id": 1, "error": { "code": -32000, "message": "...", "data": { ... } } }
```

### 2.2 Domain: Page

Navigation lifecycle. Maps to CDP's Page + partial Target.

#### Commands

| Method | Params | Returns | CDP Equivalent |
|--------|--------|---------|---------------|
| `Page.navigate` | `{url, wait?: "dom"\|"load"\|"settled"\|"idle"}` | `{url, title, state, page_id, timing}` | `Page.navigate` + wait for `loadEventFired` |
| `Page.reload` | `{ignoreCache?: bool}` | `{url, title, state, timing}` | `Page.reload` |
| `Page.back` | — | `{url, title, state}` | `Page.navigateToHistoryEntry` |
| `Page.forward` | — | `{url, title, state}` | `Page.navigateToHistoryEntry` |
| `Page.stop` | — | `{stopped: bool}` | `Page.stopLoading` |
| `Page.history` | — | `{entries: [{url, title}], current: int}` | `Page.getNavigationHistory` |
| `Page.close` | `{page_id?}` | `{closed: bool}` | `Target.closeTarget` |
| `Page.state` | — | `{url, title, state, page_id}` | — (NRP-only) |

#### Events

| Event | Params | CDP Equivalent |
|-------|--------|---------------|
| `Page.navigated` | `{url, title, state, page_id}` | `Page.frameNavigated` |
| `Page.loaded` | `{url, timing}` | `Page.loadEventFired` |
| `Page.domReady` | `{url}` | `Page.domContentEventFired` |
| `Page.settled` | `{url, idle_ms}` | — (NRP-only: no DOM mutations for N ms) |
| `Page.dialog` | `{type, message, default_prompt?}` | `Page.javascriptDialogOpening` |
| `Page.error` | `{url, error, code}` | — |

#### Types

```rust
enum PageState { Idle, Navigating, Loading, Interactive, Hydrated, Settled, Complete, Blocked, Failed }
enum WaitStrategy { Dom, Load, Settled, Idle }
```

### 2.3 Domain: Content

Structured data extraction. This is the **AI-only domain** — CDP has nothing like it. Replaces raw `DOM.getDocument` with AI-oriented output.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Content.getAXTree` | `{depth?: int, root?: node_id}` | `{tree: AXNode}` |
| `Content.getWOM` | — | `{wom: WomDocument}` |
| `Content.getText` | `{max_chars?: int, selector?: str}` | `{text: str, truncated: bool}` |
| `Content.getLinks` | `{max?: int}` | `{links: [{text, href, rel}]}` |
| `Content.getForms` | — | `{forms: [FormModel]}` |
| `Content.getTables` | `{selector?: str}` | `{tables: [{headers, rows}]}` |
| `Content.getSemantic` | `{max_tokens?: int}` | `{semantic: str}` |
| `Content.search` | `{query: str, max?: int}` | `{matches: [{node_id, text, context}]}` |

#### Events

| Event | Params |
|-------|--------|
| `Content.domChanged` | `{mutations: [{type, target_id, detail}]}` |
| `Content.formChanged` | `{form_id, field, old_value, new_value}` |

#### Types

```rust
struct AXNode {
    id: String,
    role: AXRole,              // button, textbox, link, heading, list, form, ...
    name: String,              // accessible name
    value: Option<String>,     // current value
    description: Option<String>,
    properties: AXProperties,
    children: Vec<AXNode>,
    actions: Vec<String>,      // click, type, select, expand, check, submit
}

struct AXProperties {
    disabled: bool,
    required: bool,
    checked: Option<bool>,     // None = not checkable
    selected: Option<bool>,
    expanded: Option<bool>,
    focused: bool,
    editable: bool,
    multiline: bool,
    visible: bool,             // display:none detection
    interactive: bool,         // pointer-events, disabled, etc.
    input_type: Option<String>,
    placeholder: Option<String>,
    autocomplete: Option<String>,
}

enum AXRole {
    Button, Link, Textbox, Checkbox, Radio, Select, Option,
    Heading, Paragraph, List, ListItem, Table, Row, Cell,
    Form, Navigation, Banner, Main, Complementary, Contentinfo,
    Article, Section, Dialog, Alert, Menu, MenuItem, Tab, TabPanel,
    Image, Figure, Separator, Generic,
}

struct FormModel {
    id: Option<String>,
    action: Option<String>,
    method: String,            // GET, POST
    fields: Vec<FormField>,
    submit_label: Option<String>,
    csrf_token: Option<String>,
}

struct FormField {
    node_id: String,
    name: Option<String>,
    input_type: String,
    label: String,
    value: Option<String>,
    required: bool,
    disabled: bool,
    placeholder: Option<String>,
    options: Vec<SelectOption>, // for <select>
    pattern: Option<String>,
    min: Option<String>,
    max: Option<String>,
}
```

### 2.4 Domain: Interact

AI-intent-level interaction. CDP's Input domain provides raw mouse/key events. We provide semantic actions.

#### Commands

| Method | Params | Returns | CDP Equivalent |
|--------|--------|---------|---------------|
| `Interact.click` | `{target: str}` | `{clicked: bool, caused_navigation: bool, node_id: str}` | `Input.dispatchMouseEvent` x2 (down+up) |
| `Interact.type` | `{target: str, text: str}` | `{typed: bool, value: str}` | `DOM.focus` + `Input.insertText` |
| `Interact.pressKey` | `{target: str, key: str}` | `{pressed: bool}` | `Input.dispatchKeyEvent` x2 (down+up) |
| `Interact.select` | `{target: str, value: str}` | `{selected: bool, label: str}` | Runtime.evaluate |
| `Interact.check` | `{target: str, checked: bool}` | `{checked: bool}` | Runtime.evaluate |
| `Interact.hover` | `{target: str}` | `{hovered: bool}` | `Input.dispatchMouseEvent` (move) |
| `Interact.scroll` | `{target?: str, direction: str, amount?: int}` | `{scrolled: bool}` | `Input.dispatchMouseEvent` (wheel) |
| `Interact.focus` | `{target: str}` | `{focused: bool}` | `DOM.focus` |
| `Interact.doubleClick` | `{target: str}` | `{clicked: bool}` | `Input.dispatchMouseEvent` (clickCount: 2) |
| `Interact.upload` | `{target: str, path: str}` | `{uploaded: bool, filename: str}` | `DOM.setFileInputFiles` |

Target resolution: `target` accepts WOM node `id`, CSS selector, `text:Submit`, `label:Email`, `role:button`. The `resolve` module handles all strategies.

#### Events

| Event | Params |
|-------|--------|
| `Interact.actionComplete` | `{action, target_id, duration_ms, caused_navigation}` |

### 2.5 Domain: Form

Compound form operations. CDP has nothing equivalent — Puppeteer/Playwright build this in their client libraries.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Form.extract` | `{target?: str}` | `{form: FormModel}` |
| `Form.fill` | `{target?: str, fields: {name: value}}` | `{filled: [str], skipped: [str], errors: [str]}` |
| `Form.validate` | `{target?: str}` | `{valid: bool, errors: [{field, message}]}` |
| `Form.submit` | `{target?: str}` | `{submitted: bool, caused_navigation: bool, response_status?: int}` |
| `Form.fillAndSubmit` | `{target?: str, fields: {name: value}}` | `{filled: [str], submitted: bool, page?: PageResult}` |
| `Form.detectCSRF` | `{target?: str}` | `{found: bool, token_name?: str, token_value?: str}` |

### 2.6 Domain: Wait

Condition-based waiting. CDP uses event subscriptions + client-side polling. We provide first-class wait commands.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Wait.forSelector` | `{selector: str, timeout_ms?: int}` | `{found: bool, elapsed_ms: int}` |
| `Wait.forText` | `{text: str, timeout_ms?: int}` | `{found: bool, elapsed_ms: int, node_id?: str}` |
| `Wait.forNavigation` | `{timeout_ms?: int}` | `{navigated: bool, url?: str}` |
| `Wait.forStable` | `{idle_ms?: int, timeout_ms?: int}` | `{stable: bool, mutations_seen: int}` |
| `Wait.forHidden` | `{selector: str, timeout_ms?: int}` | `{hidden: bool, elapsed_ms: int}` |
| `Wait.forNetworkIdle` | `{idle_ms?: int, timeout_ms?: int}` | `{idle: bool, pending_requests: int}` |

### 2.7 Domain: Network

HTTP observation and interception. Closely mirrors CDP's Network + Fetch.

#### Commands

| Method | Params | Returns | CDP Equivalent |
|--------|--------|---------|---------------|
| `Network.enable` | `{patterns?: [str]}` | `{enabled: bool}` | `Network.enable` |
| `Network.disable` | — | `{disabled: bool}` | `Network.disable` |
| `Network.getLog` | `{max?: int, filter?: str}` | `{entries: [NetworkLogEntry]}` | — (NRP keeps a log) |
| `Network.setHeaders` | `{headers: {name: value}}` | `{set: bool}` | `Network.setExtraHTTPHeaders` |
| `Network.intercept` | `{patterns: [str], stage?: "request"\|"response"}` | `{intercepting: bool}` | `Fetch.enable` |
| `Network.continueRequest` | `{request_id: str, url?: str, headers?: obj}` | `{continued: bool}` | `Fetch.continueRequest` |
| `Network.fulfillRequest` | `{request_id: str, status: int, headers?: obj, body?: str}` | `{fulfilled: bool}` | `Fetch.fulfillRequest` |
| `Network.failRequest` | `{request_id: str, reason: str}` | `{failed: bool}` | `Fetch.failRequest` |

#### Events

| Event | Params | CDP Equivalent |
|-------|--------|---------------|
| `Network.requestSent` | `{request_id, url, method, headers}` | `Network.requestWillBeSent` |
| `Network.responseReceived` | `{request_id, url, status, headers, timing}` | `Network.responseReceived` |
| `Network.requestPaused` | `{request_id, url, method, stage}` | `Fetch.requestPaused` |
| `Network.requestFailed` | `{request_id, url, error}` | `Network.loadingFailed` |

### 2.8 Domain: Auth

Session and authentication management. Combines CDP's Storage + Network cookie commands with AI-oriented login flow support.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Auth.getCookies` | `{domain?: str}` | `{cookies: [Cookie]}` |
| `Auth.setCookie` | `{cookie: Cookie}` | `{set: bool}` |
| `Auth.setCookies` | `{cookies: [Cookie]}` | `{set: int}` |
| `Auth.clearCookies` | `{domain?: str}` | `{cleared: int}` |
| `Auth.importCookies` | `{path: str, domain?: str}` | `{imported: int}` |
| `Auth.exportCookies` | `{path: str, domain?: str}` | `{exported: int}` |
| `Auth.getStorage` | `{origin: str, type: "local"\|"session"}` | `{items: {key: value}}` |
| `Auth.setStorage` | `{origin: str, type: str, items: {key: value}}` | `{set: int}` |

### 2.9 Domain: Runtime

JavaScript execution. Closely mirrors CDP's Runtime.

#### Commands

| Method | Params | Returns | CDP Equivalent |
|--------|--------|---------|---------------|
| `Runtime.evaluate` | `{expression: str, return_by_value?: bool, await_promise?: bool}` | `{result: Value, type: str}` | `Runtime.evaluate` |
| `Runtime.callFunction` | `{declaration: str, args?: [Value]}` | `{result: Value}` | `Runtime.callFunctionOn` |

#### Events

| Event | Params | CDP Equivalent |
|-------|--------|---------------|
| `Runtime.consoleMessage` | `{level, text, url, line}` | `Runtime.consoleAPICalled` |
| `Runtime.exception` | `{text, url, line, stack}` | `Runtime.exceptionThrown` |

### 2.10 Domain: Session

Session lifecycle. Replaces CDP's Target domain for our single-engine model.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Session.status` | — | `{state, page_id, url, title, uptime_ms}` |
| `Session.reset` | — | `{reset: bool}` |
| `Session.getConfig` | — | `{config: EngineConfig}` |
| `Session.setConfig` | `{config: Partial<EngineConfig>}` | `{applied: bool}` |

### 2.11 Domain: Observe

Event subscriptions for async monitoring. This is our version of CDP's enable/disable pattern, unified across domains.

#### Commands

| Method | Params | Returns |
|--------|--------|---------|
| `Observe.subscribe` | `{events: [str]}` | `{subscribed: [str]}` |
| `Observe.unsubscribe` | `{events: [str]}` | `{unsubscribed: [str]}` |
| `Observe.list` | — | `{subscriptions: [str]}` |

Events from any domain can be subscribed to. Example: `Observe.subscribe({events: ["Content.domChanged", "Network.requestSent"]})`.

---

## 3. AXtree Specification

### 3.1 What It Is

The AXtree (Accessibility Tree) is our replacement for CDP's pixel-rendered page. Where Chrome renders pixels and then derives an AX tree from the render tree, we build the AXtree directly from our DOM — skipping the entire rendering pipeline.

```
Chrome:   HTML → DOM → Style → Layout → Paint → Composite → AX Tree
NeoRender: HTML → DOM → JS Execute → AX Tree (direct)
```

### 3.2 How It Maps to DOM

| DOM Element | AX Role | AX Name Source | Actions |
|------------|---------|---------------|---------|
| `<button>` | Button | textContent, aria-label | click |
| `<a href>` | Link | textContent, aria-label | click (navigate) |
| `<input type=text>` | Textbox | label[for], aria-label, placeholder | type, clear |
| `<input type=checkbox>` | Checkbox | label[for], aria-label | check, uncheck |
| `<input type=radio>` | Radio | label[for], aria-label | select |
| `<select>` | Select | label[for], aria-label | select |
| `<textarea>` | Textbox (multiline) | label[for], aria-label | type, clear |
| `<h1>`-`<h6>` | Heading | textContent | — |
| `<p>`, `<span>`, `<div>` | Generic/Paragraph | textContent | — |
| `<img>` | Image | alt, aria-label | — |
| `<form>` | Form | aria-label, heading within | submit |
| `<nav>` | Navigation | aria-label | — |
| `<main>` | Main | — | — |
| `<table>` | Table | caption, aria-label | — |
| `<ul>`, `<ol>` | List | — | — |
| `<li>` | ListItem | textContent | — |
| `<dialog>` | Dialog | aria-label, heading within | — |
| `[role=X]` | X | aria-label, aria-labelledby | per role |

### 3.3 Accessible Name Computation (simplified)

Priority order:
1. `aria-labelledby` → concatenate referenced elements' text
2. `aria-label`
3. `<label for="id">` association
4. `placeholder` (for inputs)
5. `alt` (for images)
6. `title` attribute
7. Direct text content (for buttons, links, headings)

### 3.4 Visibility Detection (without layout)

Since we don't compute layout, visibility is determined by:
- `display: none` in inline style → hidden
- `visibility: hidden` in inline style → hidden
- `hidden` attribute → hidden
- `aria-hidden="true"` → hidden from AX tree
- `type="hidden"` on inputs → hidden
- `opacity: 0` in inline style → treat as hidden

For external stylesheets, we cannot detect `display:none` without CSS computation. This is a known limitation. Sites that hide elements purely via external CSS will show those elements in our AXtree. This is acceptable — the AI gets more information, not less.

### 3.5 Relationship to WOM

WOM (Web Object Model) is our **current** AI-optimized page representation (flat list of `WomNode`s). The AXtree is the **next evolution**: a hierarchical tree with proper parent-child relationships, richer role vocabulary, and standard accessibility semantics.

Migration path:
1. `WomDocument` continues to work as-is
2. `Content.getAXTree` returns the new hierarchical tree
3. `Content.getWOM` returns the flat WOM (backward compatible)
4. Eventually, WOM becomes a flattened view derived from the AXtree

---

## 4. What We Strip From CDP and Why

| Category | Stripped Domains | Reason |
|----------|-----------------|--------|
| **Visual rendering** | Animation, Overlay, LayerTree, HeadlessExperimental | We don't render pixels. No paint, no composite, no GPU. The AXtree is our output. |
| **Profiling** | Performance, PerformanceTimeline, Profiler, HeapProfiler, Memory, Tracing | Developer tooling for optimizing web apps. AI agents don't profile. |
| **Media** | Media, WebAudio, Cast | A/V playback inspection. AI agents read content, not play media. |
| **Developer debugging** | Debugger, DOMDebugger, EventBreakpoints, Inspector | Breakpoints, stepping, watches — human developer tools. |
| **Browser internals** | Browser, SystemInfo, Schema, IO, Tethering | Chrome-specific plumbing. We are the browser. |
| **Specialized APIs** | ServiceWorker, BackgroundService, PWA, WebAuthn, DeviceAccess, DeviceOrientation, IndexedDB, CacheStorage, Database | Niche browser features AI agents don't need for web interaction. |
| **Chrome features** | Extensions, Autofill, Preload, Audits | Chrome-specific functionality. |

Total: 33 domains stripped. They represent ~3000+ commands/events we don't need to implement.

---

## 5. What We ADD That CDP Doesn't Have

| NRP Feature | Why CDP Lacks It | Value for AI |
|------------|-----------------|-------------|
| **WOM / AXtree from DOM** | CDP needs a rendered page to produce AX tree | We build it directly, no rendering cost |
| **Content.search** | CDP has no page search command | AI can find text in page without JS |
| **Form.extract** | CDP has no form understanding | AI gets form model with all fields, types, labels, validation |
| **Form.fill** | CDP requires field-by-field Input events | One command fills entire form |
| **Form.fillAndSubmit** | CDP needs 10+ commands | One command: fill + validate + submit |
| **Form.detectCSRF** | CDP has no CSRF awareness | AI gets token auto-detected |
| **Wait.forStable** | CDP has no "DOM settled" concept | Critical for SPAs — wait until JS finishes mutating |
| **Wait.forText** | CDP has no text wait | AI waits for content, not selectors |
| **Content.getSemantic** | CDP returns raw DOM | Token-efficient page overview for AI context |
| **Page.settled event** | CDP has load/DOMContentLoaded only | SPA-aware lifecycle |
| **Interact with node IDs** | CDP needs coordinates or node IDs | AI uses semantic targets ("Submit button") |
| **Network.getLog** | CDP is event-only | AI can query request history |

---

## 6. Comparison: CDP vs NRP

| Aspect | CDP | NRP |
|--------|-----|-----|
| **Audience** | Human developers | AI agents |
| **Output** | Pixels + raw DOM | AXtree + WOM |
| **Transport** | WebSocket only | stdio (MCP) + WebSocket |
| **Domains** | 48 | 10 |
| **Granularity** | Low-level (mouse at x,y) | Intent-level (click "Submit") |
| **Form handling** | Manual (focus, type each char, click submit) | `Form.fillAndSubmit({fields})` |
| **Page readiness** | `loadEventFired` (insufficient for SPAs) | `Page.settled` (DOM mutation idle) |
| **Content extraction** | `DOM.getDocument` → raw tree | `Content.getWOM` → AI-structured |
| **Rendering required** | Yes (full Blink pipeline) | No (DOM + JS only) |
| **Resource cost** | ~300MB+ per tab | ~50MB per page |
| **Latency** | 2-5s per page | 200ms-2s per page |
| **Concurrency** | 2-3 tabs practical | 10+ parallel pages |
| **Commands to fill & submit form** | ~15-20 (focus, type x N, click) | 1 (`Form.fillAndSubmit`) |

---

## 7. Implementation Plan

### 7.1 What Already Exists

| Component | Status | NRP Domain it becomes |
|-----------|--------|-----------------------|
| `neo-engine::BrowserEngine` trait | Working | Basis for all NRP domains |
| `BrowserEngine::navigate()` | Working | `Page.navigate` |
| `BrowserEngine::click()` | Working | `Interact.click` |
| `BrowserEngine::type_text()` | Working | `Interact.type` |
| `BrowserEngine::fill_form()` | Working | `Form.fill` |
| `BrowserEngine::submit()` | Working | `Form.submit` |
| `BrowserEngine::eval()` | Working | `Runtime.evaluate` |
| `BrowserEngine::extract()` (WOM) | Working | `Content.getWOM` |
| `BrowserEngine::extract_text()` | Working | `Content.getText` |
| `BrowserEngine::extract_links()` | Working | `Content.getLinks` |
| `BrowserEngine::extract_semantic()` | Working | `Content.getSemantic` |
| `BrowserEngine::wait_for()` | Working | `Wait.forSelector` |
| `BrowserEngine::press_key()` | Working | `Interact.pressKey` |
| `neo-mcp` tools: browse, interact, extract, eval, wait, search, trace, import_cookies | Working | MCP layer above NRP |
| `neo-extract::WomDocument` / `WomNode` | Working | Basis for `Content.getWOM`, evolves into AXtree |
| `neo-interact` module (click, type, forms, keyboard, scroll, etc.) | Working | Backend for Interact + Form domains |
| `neo-http` (rquest client) | Working | Backend for Network domain |
| Cookie import from Chrome SQLite | Working | `Auth.importCookies` |

### 7.2 What to Build

| Component | Crate | Effort | Priority |
|-----------|-------|--------|----------|
| **NRP types** — `NrpCommand`, `NrpResponse`, `NrpEvent`, `BackendSource`, domain enums | `neo-types` | 2 days | P0 |
| **AXtree builder** — DOM → AXNode tree with roles, names, properties | `neo-extract` | 1 week | P0 |
| **NRP dispatcher** — JSON-RPC command parsing, domain routing, response envelope | `neo-engine` (new module `nrp/`) | 3 days | P0 |
| **Event system** — subscribe, fire, deliver events (DOM mutations, navigation, network) | `neo-engine` (new module `events/`) | 3 days | P1 |
| **Form domain** — `extract`, `fill`, `validate`, `submit`, `fillAndSubmit`, `detectCSRF` | `neo-interact` (extend) | 2 days | P1 |
| **Wait extensions** — `forText`, `forStable`, `forNavigation`, `forHidden`, `forNetworkIdle` | `neo-engine` (extend) | 2 days | P1 |
| **Network observation** — request log, enable/disable, event emission | `neo-http` (extend) | 2 days | P1 |
| **Network interception** — pause/continue/fulfill/fail requests | `neo-http` (new) | 3 days | P2 |
| **Auth domain** — cookie CRUD, storage CRUD, import/export | `neo-engine` (new module `auth/`) | 1 day | P1 |
| **MCP rewrite** — tools become thin wrappers over NRP commands | `neo-mcp` | 2 days | P1 |
| **WebSocket transport** — NRP over WS for remote agents | new crate `neo-ws` | 3 days | P2 |
| **Content.search** — full-text search in DOM | `neo-extract` (extend) | 1 day | P2 |

### 7.3 File Changes

```
neo-types/src/
  lib.rs                          # add NrpCommand, NrpResponse, NrpEvent, AXRole, etc.
  nrp.rs                          # NEW: all NRP protocol types

neo-extract/src/
  axtree.rs                       # NEW: DOM → AXtree builder
  axtree_names.rs                 # NEW: accessible name computation
  wom.rs                          # keep, add conversion to/from AXtree
  delta.rs                        # extend for AXtree diffing

neo-engine/src/
  nrp/mod.rs                      # NEW: NRP command dispatcher
  nrp/page.rs                     # NEW: Page domain handler
  nrp/content.rs                  # NEW: Content domain handler
  nrp/interact.rs                 # NEW: Interact domain handler
  nrp/form.rs                     # NEW: Form domain handler
  nrp/wait.rs                     # NEW: Wait domain handler
  nrp/network.rs                  # NEW: Network domain handler
  nrp/auth.rs                     # NEW: Auth domain handler
  nrp/runtime.rs                  # NEW: Runtime domain handler
  nrp/session.rs                  # NEW: Session domain handler
  nrp/observe.rs                  # NEW: Observe domain handler
  events.rs                       # NEW: event subscription + delivery

neo-mcp/src/
  tools/*.rs                      # rewrite: call NRP dispatcher instead of BrowserEngine directly

neo-http/src/
  observe.rs                      # NEW: request logging, event emission
  intercept.rs                    # NEW: request interception pipeline
```

### 7.4 Timeline

| Week | Deliverable |
|------|-------------|
| **Week 1** | NRP types + AXtree builder + Page/Content domains working |
| **Week 2** | Interact + Form + Wait domains + MCP rewrite |
| **Week 3** | Network + Auth + Runtime + Session + Event system |
| **Week 4** | Testing, edge cases, WebSocket transport |

### 7.5 How MCP Tools Map to NRP Commands

| Current MCP Tool | Current Method | NRP Command |
|-----------------|---------------|-------------|
| `browse(url)` | `engine.navigate(url)` + `engine.extract()` | `Page.navigate` (returns WOM automatically) |
| `interact(click, target)` | `engine.click(target)` | `Interact.click({target})` |
| `interact(type, target, text)` | `engine.type_text(target, text)` | `Interact.type({target, text})` |
| `interact(fill_form, fields)` | `engine.fill_form(fields)` | `Form.fill({fields})` |
| `interact(submit, target)` | `engine.submit(target)` | `Form.submit({target})` |
| `interact(press_key, target, key)` | `engine.press_key(target, key)` | `Interact.pressKey({target, key})` |
| `extract(wom)` | `engine.extract()` | `Content.getWOM` |
| `extract(text)` | `engine.extract_text()` | `Content.getText` |
| `extract(links)` | `engine.extract_links()` | `Content.getLinks` |
| `extract(semantic)` | `engine.extract_semantic()` | `Content.getSemantic` |
| `extract(tables)` | custom extraction | `Content.getTables` |
| `eval(expression)` | `engine.eval(js)` | `Runtime.evaluate({expression})` |
| `wait(selector)` | `engine.wait_for(selector)` | `Wait.forSelector({selector})` |
| `search(query)` | DDG fetch + parse | stays as MCP tool (not NRP — external HTTP) |
| `trace()` | `engine.trace()` | `Session.status` + trace data |
| `import_cookies(path)` | file read + inject | `Auth.importCookies({path})` |

MCP tools become thin wrappers: parse args, call `nrp_dispatch(command)`, format response as MCP content blocks.

### 7.6 Future: REPL over NRP

A REPL can be built as another transport for NRP:

```
neorender> Page.navigate https://example.com
{ok: true, page: {url: "https://example.com", title: "Example Domain", state: "settled"}, timing: {total_ms: 245}}

neorender> Content.getAXTree
{tree: {role: "Main", children: [{role: "Heading", name: "Example Domain"}, ...]}}

neorender> Interact.click "More information..."
{clicked: true, caused_navigation: true}
```

Same protocol, different transport (stdin line-by-line with pretty printing).

---

## 8. Migration Path

### Phase 1: NRP Types (non-breaking)

Add `neo-types/src/nrp.rs` with all protocol types. No existing code changes.

### Phase 2: AXtree Builder (non-breaking)

Add `neo-extract/src/axtree.rs`. WOM continues to work. AXtree is a new extraction mode.

### Phase 3: NRP Dispatcher (non-breaking, behind feature flag)

Add `neo-engine/src/nrp/`. The dispatcher calls existing `BrowserEngine` methods internally. Both paths coexist.

### Phase 4: MCP Rewire (feature flag)

MCP tools call NRP dispatcher instead of `BrowserEngine` directly. `--features nrp` enables new path.

### Phase 5: Default + Cleanup

NRP becomes the default path. Old direct `BrowserEngine` calls from MCP are removed. `BrowserEngine` trait stays as the internal engine interface — NRP domains are the external protocol.

```
Before:  MCP tool → BrowserEngine → DOM/JS/HTTP
After:   MCP tool → NRP dispatcher → BrowserEngine → DOM/JS/HTTP
         WS client → NRP dispatcher → BrowserEngine → DOM/JS/HTTP
         REPL → NRP dispatcher → BrowserEngine → DOM/JS/HTTP
```

The protocol layer is the single entry point. All consumers (MCP, WebSocket, REPL) speak NRP.
