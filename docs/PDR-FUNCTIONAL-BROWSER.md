# PDR: Functional Browser — From "Loads Pages" to "Actually Works"

## State of the art
V2 can navigate 11/11 sites, extract content, fill forms on static sites, bypass Cloudflare. But it CANNOT interact with SPAs because frameworks don't hydrate. The browser loads HTML + JS but the app never "activates".

## The actual gap
Modern web = SPA. If the SPA doesn't mount, the browser is a fancy wget.

Evidence from ChatGPT:
- 3MB of modules load without errors ✅
- `__reactRouterContext` with streaming SSR data exists ✅
- `$RC` (React completeBoundary) function exists ✅
- But: `typeof React === 'undefined'`, no fibers on DOM elements
- Conclusion: modules LOAD but the initialization chain STOPS somewhere

## Root cause investigation plan

### I1: Module evaluation completion
deno_core loads modules but may not fully evaluate them if they contain:
- Top-level await
- Circular dependencies that deadlock
- Dynamic import() that resolves but whose callbacks don't run

**Test**: After module load, check if mod_evaluate promise resolved:
```rust
// In module loader, after loading a dynamic import:
// Log: did the evaluation promise resolve? Or is it still pending?
```

**Action**: Read `crates/neo-runtime/src/modules.rs` and `v8_runtime_impl.rs` to understand how module evaluation works. Check if we await the evaluation future completely.

### I2: Event loop drainage depth
Current pump: 50 microtask + 20 macrotask rounds. But ChatGPT's init chain may need MORE:
- Module loads → schedules microtask → microtask loads another module → schedules another microtask → ...
- This chain could be 100+ levels deep

**Test**: Pump for longer (500ms instead of 100ms) and count rounds:
```rust
let budget = Duration::from_millis(500);
// Count total rounds, log if still doing work when budget expires
```

**Action**: Temporarily increase pump budget in pipeline.rs settle phase to 500ms. Re-test ChatGPT. If more nodes appear → the pump was too short.

### I3: Promises that never resolve
A single unresolved promise can block the entire chain. Common causes:
- `fetch()` that never completes (our op_fetch is sync, should be fine)
- `setTimeout` that never fires (our timer budget may exhaust)
- `requestAnimationFrame` callback lost
- `IntersectionObserver` callback that never fires (ours fires immediately, should be ok)
- A promise waiting for a DOM event that never happens

**Test**: Inject a global unhandled rejection handler + promise tracking:
```javascript
let __promiseCount = 0;
let __resolvedCount = 0;
const OrigPromise = Promise;
// Can't easily wrap Promise constructor, but can track rejections
```

Actually simpler: after all scripts + pump, check what's still pending via eval.

### I4: Missing APIs that silently break init
The init chain calls an API we don't have → returns undefined → next step fails silently.

**Test**: Temporarily wrap critical APIs with logging:
```javascript
// In browser_shim.js, when NEORENDER_TRACE=1:
const _origWS = window.WebSocket;
window.WebSocket = function() {
    console.log('[MISSING] WebSocket constructor called');
    throw new Error('WebSocket not available');
};
```

Better approach: create a Proxy-based trap for common missing APIs.

### I5: CSS/style dependency
Some React apps check `getComputedStyle` or `window.innerWidth` before mounting. Our stubs return defaults but maybe the specific values cause a different code path.

**Test**: Check if any script accesses style-related APIs:
```javascript
// Wrap getComputedStyle with logging
const _origGCS = getComputedStyle;
getComputedStyle = function(el) {
    console.log('[STYLE] getComputedStyle called on', el.tagName);
    return _origGCS(el);
};
```

## Implementation: Diagnostic mode

### Phase 0: Build diagnostic tooling

Create `neorender diagnose <url>` CLI command that:

1. Navigates to URL with max verbosity
2. After all scripts + modules load, reports:
   - Total module count loaded
   - Any module evaluation errors/rejections
   - Timer count (pending/fired/exhausted)
   - Fetch count (pending/completed/failed)
   - DOM node count progression: after parse, after scripts, after settle, after extended pump
   - Framework detection
   - Specific React/Vue/Svelte mount markers checked:
     - React: `__reactFiber` on any element, `_reactRootContainer`, `__REACT_DEVTOOLS_GLOBAL_HOOK__`
     - Vue: `__vue_app__`, `__VUE__`
     - Svelte: `__svelte`
   - List of globals that changed after script execution
   - Any console.error output captured
   - Any unhandled rejections captured

3. Runs extended pump (500ms budget) and reports DOM delta

4. Outputs structured JSON:
```json
{
  "url": "https://chatgpt.com",
  "modules_loaded": 4,
  "module_errors": 0,
  "dom_nodes_after_parse": 27,
  "dom_nodes_after_scripts": 319,
  "dom_nodes_after_settle": 323,
  "dom_nodes_after_extended_pump": 323,
  "framework": "react-router",
  "react_mounted": false,
  "react_fibers_found": 0,
  "console_errors": [],
  "unhandled_rejections": [],
  "pending_timers": 0,
  "pending_fetches": 0,
  "globals_added": ["__reactRouterContext", "__oai_SSR_HTML", ...],
  "missing_api_accesses": []
}
```

### Phase 1: Fix based on diagnosis

Whatever the diagnosis reveals. Possible fixes ranked by probability:

**Most likely (70%): event loop needs more drainage**
- Increase settle budget
- Or: pump in a loop until DOM stops changing (not fixed budget)

**Likely (50%): module evaluation incomplete**
- deno_core's dynamic import evaluation may need explicit event loop pumping between module loads
- Check: after `load()` returns, is the module's top-level code fully executed? Or is it queued?

**Possible (30%): missing API in late init**
- WebSocket, Worker, or other API that the app tries to use during init
- Fix: add targeted stub

**Unlikely (10%): CSS/layout dependency**
- App checks dimensions before mounting
- Fix: better stubs for innerWidth/offsetHeight

### Phase 2: Generalize

After ChatGPT works, test on:
- Mercadona.es (Vue/Nuxt — needs API data but script should execute)
- nuxt.com (Nuxt 3)
- svelte.dev (SvelteKit)
- vercel.com (already 0 errors, 86 nodes — check if React mounted)

### Phase 3: Connect as daily-use MCP

Once interaction works on 3+ SPAs:
- Register V2 as MCP server in ~/.claude.json
- Replace V1 neobrowser for daily use
- Monitor failures, add stubs as needed

## Gate

### Phase 0 gate
- `neorender diagnose <url>` produces structured JSON
- ChatGPT diagnosis reveals exact blocker (event loop? module eval? API?)

### Phase 1 gate
- ChatGPT: React fibers appear on DOM elements (mount succeeded)
- OR: documented impossible reason (e.g. needs real CSS layout)

### Phase 2 gate
- 3+ SPAs mount frameworks
- Interaction triggers framework handlers (not just DOM events)

### Phase 3 gate
- V2 registered as MCP
- Used in real conversation to navigate a site
