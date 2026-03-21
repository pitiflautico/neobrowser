# PDR: NeoRender V2 MVP — Functional AI Browser

## Status Quo
- 10 crates, ~280 tests, pipeline 9/9
- 8/10 real sites load, content extraction works
- Chrome cookie import + auto-import works
- `neorender see <url>` — read-only navigation
- `neorender search <query>` — DDG search with deep mode
- All interaction modules exist (click, type, keyboard, forms, scroll) but only work on static parsed DOM, NOT on live V8-rendered pages

## Gap: No Live Interaction
V2 can SEE the web but cannot ACT on it. The interaction modules (neo-interact) operate on neo-dom's html5ever DOM, which is the parsed HTML. But after V8 executes JavaScript, the live DOM is inside linkedom (V8). There's no bridge to mutate the live DOM and re-extract.

## MVP Requirements

### M1: Live DOM Bridge
Connect neo-interact to the V8-rendered DOM (linkedom), not just html5ever.
- After V8 execution, the DOM lives in linkedom inside V8
- Need: `eval("document.querySelector('#login').value = 'user@email.com'")` style interaction
- The JsRuntime trait already has `eval()` and `execute()`
- Bridge: translate neo-interact operations into JS eval calls on the live DOM

### M2: Multi-Step Session
Current `see` does: fetch → parse → execute JS → extract → done. One shot.
Need: navigate → interact → navigate → interact... (session loop).
- NeoSession must support `navigate()` → `interact()` → `navigate()` cycles
- DOM state persists between interactions
- Cookies persist across navigations (already done)

### M3: MCP Tools (Live)
Wire these MCP tools to the live engine:

| Tool | Action | Priority |
|------|--------|----------|
| `browse` | Navigate to URL, return WOM | P0 |
| `click` | Click element by selector/text | P0 |
| `type` | Type text into input | P0 |
| `submit` | Submit form | P0 |
| `extract` | Get WOM/text/links from current page | P0 |
| `scroll` | Scroll page | P1 |
| `screenshot` | Not applicable (headless) | N/A |
| `download` | Save response body to file | P1 |
| `import_cookies` | Import from Chrome (done) | ✅ |
| `search` | DDG search (done) | ✅ |
| `fill_form` | Fill multiple fields + submit | P0 |
| `keyboard` | Press keys (Enter, Tab, Escape) | P1 |
| `wait` | Wait for DOM condition | P1 |

### M4: ChatGPT Pong
The acid test: send a message to ChatGPT and get a response.
1. Navigate to chatgpt.com (cookies auto-imported)
2. Find the textarea (#prompt-textarea)
3. Type a message
4. Click submit button
5. Wait for response to appear
6. Extract the assistant's reply

### M5: Form Benchmark
Test on real forms:
- Google search (type query + submit)
- Login pages (fill email/password + submit)
- Contact forms (fill fields + submit)

## Architecture

### Live DOM Bridge (M1)
```
AI intent: "type 'hello' in #search"
    ↓
neo-interact: type_text(dom, "#search", "hello")
    ↓
LiveDomAdapter (NEW): translates to JS
    ↓
JsRuntime.eval("document.querySelector('#search').value = 'hello';
                document.querySelector('#search').dispatchEvent(new Event('input'))")
    ↓
V8/linkedom executes → DOM updated
    ↓
Re-extract WOM from updated DOM
```

Create `crates/neo-engine/src/live_dom.rs`:
- `LiveDomAdapter` wraps a `&mut dyn JsRuntime`
- Implements key operations as JS eval:
  - `click(selector)` → `el.click()`
  - `type_text(selector, text)` → `el.value = text; el.dispatchEvent(...)`
  - `submit(selector)` → `el.closest('form').submit()` or `el.click()`
  - `get_value(selector)` → `el.value || el.textContent`
  - `query(selector)` → `document.querySelector(selector)?.outerHTML`
  - `wait_for(selector, timeout)` → poll with setTimeout
  - `extract_wom()` → serialize current DOM to WOM JSON

### Session Loop (M2)
```
NeoSession {
    state: SessionState,  // Idle, Navigated, Interacting
    runtime: Box<dyn JsRuntime>,  // persists across interactions
    cookies: Box<dyn CookieStore>,

    navigate(url) → PageResult,
    click(selector) → InteractResult,
    type_text(selector, text) → InteractResult,
    submit(selector) → SubmitResult,  // may trigger navigation
    extract() → WomResult,
    eval(js) → String,
}
```

### MCP Server (M3)
The MCP server holds a `NeoSession` and dispatches tool calls:
```
browse(url) → session.navigate(url) → WOM
click(selector) → session.click(selector) → result
type(selector, text) → session.type_text(selector, text) → result
submit(selector) → session.submit(selector) → result (may re-navigate)
extract(what) → session.extract() → WOM/text/links
fill_form(fields) → for each field: session.type_text() → session.submit()
```

## Phases

### Phase 1: Live DOM Bridge (M1)
- Create `LiveDomAdapter` in neo-engine
- JS eval functions for click, type, submit, extract
- Tests with V8 runtime + simple HTML

### Phase 2: Session Loop (M2)
- Extend NeoSession with interact methods
- State machine: Idle → Navigated → Interacting
- Re-extract after each interaction
- Handle navigation triggered by submit/click

### Phase 3: MCP Tools (M3)
- Wire browse, click, type, submit, extract, fill_form
- Session persists across tool calls
- Error handling: element not found, timeout, navigation

### Phase 4: Pong Test (M4)
- ChatGPT pong via MCP tools
- Google search via MCP tools

### Phase 5: Form Benchmark (M5)
- Test on 5+ real forms
- Document results

## GPT Review Feedback (incorporated)

### 1. Typed Eval Bridge (not ad-hoc strings)
Single JS dispatcher function injected into V8. Commands serialized as typed JSON.
Prevents injection/escaping issues.

### 2. Real Interaction Events (compatible, not dogmatic)
- click: focus → pointerdown → mousedown → pointerup → mouseup → click
- type: focus → (per char: keydown → input → keyup) → change. keypress OPTIONAL (many sites ignore it, some break with it). Sequence must be compatible, not rigid.
- submit: blur current → submit event

### 3. Deterministic Waits
- `wait_for_selector(sel, timeout)` — poll for element existence
- `wait_for_text(sel, text, timeout)` — poll for text content match
- `wait_for_stable(timeout, interval_ms)` — no DOM mutations for N consecutive ms (NOT "no pending tasks" — that may never resolve on real sites)
- `wait_for_navigation(timeout)` — detect URL change after action
- `wait_for_idle` EXCLUDED from gate — unreliable on real sites. Use case-specific waits instead.

### 4. Post-Action Navigation Model
```
enum ActionOutcome {
    NoOp,            // nothing changed
    DomMutation,     // DOM changed, same page
    JsOnlyEffect,   // JS executed but no visible DOM mutation (e.g. state update)
    SpaRouteChange,  // URL changed without reload
    FullNavigation,  // full page reload
    NewContext,       // popup/new tab opened (DETECTED but not followed in MVP)
}
```

### 5. Robust Element Targeting
Fallback chain: CSS selector → text content → role → aria-label.
Post-resolution filters: visible, enabled, editable (for inputs).
Skip hidden/disabled elements. If ambiguous → error with candidates list.

### 6. Handle Re-resolution
Never cache DOM refs. Re-query every operation.

### 7. Error Contract
Every operation returns: value + outcome + mutations + elapsed_ms + warnings.
Errors:
- NotFound — element doesn't exist
- AmbiguousMatch — multiple candidates, can't disambiguate
- NotInteractable — element exists but hidden/disabled/readonly
- Timeout — wait exceeded deadline
- DetachedNode — element removed between resolve and action
- NavigationAborted — navigation started but failed/redirected
- JsException — V8 threw during eval
- CrossOriginFrame — tried to access cross-origin iframe (detected, not supported)

### 8. Frames/Shadow DOM
MVP scope: frame access via selector/name/id + frame discovery (list frames).
Default frame = top. Shadow DOM: EXCLUDED (documented).
Popups/new windows: DETECTED (NewContext outcome) but not followed in MVP.

### 9. Wait Strategy for Dynamic Responses
Define success condition for each operation:
- Form submit: URL change OR new content matching pattern
- ChatGPT: new `[data-message-author-role="assistant"]` element
- Search: results container populated

### 10. Concrete Benchmark
| Test | URL | Action | Pass: fields filled | Pass: submit fired | Pass: outcome detected | Pass: response observable |
|------|-----|--------|--------------------|--------------------|----------------------|--------------------------|
| Google search | google.com | type "rust lang" + submit | input.value == "rust lang" | form submitted | SpaRouteChange or FullNavigation | page text contains results |
| DuckDuckGo | duckduckgo.com | type "test" + submit | input.value == "test" | form submitted | FullNavigation | results with titles |
| GitHub login | github.com/login | fill email+pass | both fields have values | submit clicked | outcome detected (any) | login error message visible |
| HN login | news.ycombinator.com/login | fill user+pass | both fields filled | form submitted | outcome detected | response page loaded |
| ChatGPT pong | chatgpt.com | type+submit+wait | textarea filled | send clicked | DomMutation | EXTERNAL benchmark (not core gate) |

## Gate (Core)
- 3+ forms: fields filled correctly, submit fired, outcome detected, response observable
- All waits work (selector, text, stable, navigation)
- Error contract enforced — all 8 error types properly raised
- All existing ~280 tests still pass
- Targeting resolves visible+enabled elements, skips hidden/disabled

## Gate (External — not blocking)
- ChatGPT pong (stress test, depends on UI changes/auth/rate limits)
- Amazon/Twitter (known anti-bot, expected failures)
