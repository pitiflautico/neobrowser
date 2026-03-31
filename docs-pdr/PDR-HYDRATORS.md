# PDR: Runtime Correctness — Make Frameworks Hydrate Without Patches

## Problem (redefined)
ChatGPT (Next.js) doesn't hydrate. Mercadona.es (SPA) fails. The temptation is to write per-framework patches. That's wrong.

A real browser doesn't know about React or Vue. It just executes JS correctly. If our runtime is correct enough, frameworks hydrate on their own.

Our runtime is NOT correct enough. Specific gaps:
- ES module import graph incomplete (dynamic import() may fail)
- Script execution order wrong (defer/async/module semantics)
- ReadableStream is a no-op (RSC streaming breaks)
- MutationObserver untested (React reconciler depends on it)
- Event loop model simplified (microtask/macrotask ordering)

## Strategy
Fix the runtime, not the frameworks. Framework detection stays ONLY as telemetry/debugging (which framework is this site using? → helps diagnose which runtime gap caused the failure).

## Current Runtime Gaps

### What works
- V8 execution (deno_core 0.311) ✅
- linkedom DOM (querySelector, createElement, events) ✅
- Sync fetch via op_fetch ✅
- Timers (setTimeout/setInterval with budget) ✅
- localStorage/sessionStorage ✅
- Navigation interception (form.submit, location) ✅
- Cookie persistence ✅

### What's broken or incomplete

| Gap | Impact | Evidence |
|-----|--------|----------|
| **Dynamic import()** | Next.js code-splits everything | ChatGPT entry module fails |
| **ReadableStream (real, not no-op)** | RSC streaming, fetch body streaming | pipeThrough returns self (hack) |
| **Script execution order** | defer runs before DOMContentLoaded, async runs when ready, module deferred by default | We execute all scripts sequentially, ignoring defer/async/module attributes |
| **MutationObserver fidelity** | React reconciler, Vue reactivity | Untested in linkedom — may not fire for all mutation types |
| **DOMContentLoaded / load events** | Many scripts wait for these | We dispatch them but timing may be wrong |
| **import maps** | SvelteKit, modern apps | `<script type="importmap">` not parsed |
| **CSS Object Model (partial)** | getComputedStyle for visibility checks | Currently returns stubs — scripts that check display:none fail |
| **Web Workers** | Some frameworks offload work | Not supported, scripts crash on `new Worker()` |
| **Top-level await in modules** | Modern ESM | May hang in deno_core module evaluation |

## Tiers

### T1: ES Module Correctness
The biggest gap. Modern apps use hundreds of ES modules with dynamic import().

**What to fix:**
- `import()` dynamic — must resolve, fetch, and execute modules on demand
- Module resolution policy (EXPLICIT):
  - Relative paths (`./foo.js`, `../bar.js`) → resolve against importer URL
  - Absolute URLs (`https://cdn.example.com/foo.js`) → use directly
  - Bare specifiers (`react`, `vue`) → REQUIRE import map. No Node-style resolution.
  - If bare specifier and no import map → error with clear message
- `<script type="importmap">` — parse JSON, use for bare specifier → URL mapping
- Module evaluation order: dependencies before dependents
- Re-exports and circular dependencies (V8 handles this, but our loader may not)
- `import.meta.url` — must return the module's URL

**How to verify:**
- Read `crates/neo-runtime/src/modules.rs` (NeoModuleLoader) — understand current gaps
- Read V1's module loader for comparison
- Create test: page with 3 ES modules importing each other + dynamic import()
- Create test: page with import map + bare specifier

**Gate:**
- Synthetic page with `import('./module.js')` dynamically loads and executes
- Import map resolves bare specifiers
- ChatGPT's entry module loads without error

### T2: Script Execution Order
HTML spec defines precise ordering for scripts.

**Current behavior:** We parse ALL HTML first, then extract all scripts, fetch externals, execute in document order. This means we CANNOT replicate true "blocking inline script during parse" because parsing is already complete.

**Documented approximation:** Since html5ever parses the full document upfront, we approximate the spec:

**Spec behavior (ideal, not fully achievable):**
```
1. Inline scripts: execute immediately when encountered (blocking)
2. External scripts (no defer/async): fetch → execute (blocking, in order)
3. defer scripts: fetch in parallel → execute in document order AFTER parsing
4. async scripts: fetch in parallel → execute as soon as ready (no order guarantee)
5. module scripts: like defer (fetched in parallel, execute after parsing, in order)
6. async module scripts: like async (execute when ready)
```

**What to fix:**
- Parse `defer`, `async`, `type="module"` attributes on each script
- Group scripts into: blocking, defer, async, module, async-module
- Execute in correct order: blocking during parse → defer after parse → async when ready
- Dispatch `DOMContentLoaded` after all defer/module scripts execute
- Dispatch `load` after all scripts + subresources

**Gate:**
- Test page with mixed script types: verify execution order matches spec
- `DOMContentLoaded` fires at correct time (after defer, before load)

### T3: Streams + Fetch Body
ReadableStream is critical for React Server Components (RSC) and modern fetch usage.

**Current state:** pipeThrough returns self (no-op). TransformStream is basic.

**What to fix:**
- ReadableStream: real implementation with reader/controller pattern
- Response.body as ReadableStream (fetch responses should be streamable)
- TextDecoderStream / TextEncoderStream (for text streaming)
- ReadableStream.pipeTo() — pipe to WritableStream
- ReadableStream.pipeThrough() — REAL implementation (not no-op)

**Constraint:** Full streaming with backpressure is complex. MVP: implement enough that frameworks can consume fetch response bodies as streams. Backpressure can be ignored (buffer everything, stream from buffer).

**Warning:** This may NOT be sufficient for Next.js App Router RSC. RSC uses ReadableStream + TransformStream with specific chunk encoding (flight protocol). If MVP streams don't work, the next step is understanding the flight chunk format — but that's a diagnostic step, not a framework patch.

**Gate:**
- `fetch(url).then(r => r.body.getReader())` works, reads chunks
- React RSC flight data streams through pipeThrough without deadlock
- Next.js App Router streaming SSR receives chunks

### T4: DOM Mutation Fidelity
React's reconciler and Vue's reactivity depend on predictable DOM mutations.

**What to verify (linkedom audit):**
1. `MutationObserver` fires on `setAttribute()` — YES/NO?
2. `MutationObserver` fires on `textContent = "..."` — YES/NO?
3. `MutationObserver` fires on `appendChild()` / `removeChild()` — YES/NO?
4. `MutationObserver` respects `{ subtree: true }` config — YES/NO?
5. `MutationObserver` batches mutations correctly (fires async, not sync) — YES/NO?
6. `el.dispatchEvent(new Event('...'))` bubbles correctly — YES/NO?

**If any fail:** Implement a MutationObserver polyfill that wraps linkedom's DOM methods (setAttribute, textContent setter, appendChild, removeChild, insertBefore) with mutation record generation.

**Gate:**
- All 6 checks pass
- React-style reconciler test: rapidly mutate DOM → observer receives all mutations

### T5: Event Loop Model
Correct ordering: microtasks (Promise.then) before macrotasks (setTimeout).

**Current state:** V8 handles microtasks natively. Our macrotask scheduling uses thread::sleep. But the interleaving may not be correct.

**What to verify:**
```javascript
// Expected output: 1, 2, 3, 4
console.log(1);
setTimeout(() => console.log(4), 0);
Promise.resolve().then(() => console.log(2));
queueMicrotask(() => console.log(3));
```

**What to fix if wrong:**
- Ensure V8 microtask queue drains before any macrotask fires
- `queueMicrotask` must run before next setTimeout
- `requestAnimationFrame` fires once per "frame" (after microtasks, before next macrotask batch)
- `MutationObserver` callbacks fire as microtasks
- Drain model must be explicit:
  1. Execute script/callback
  2. Drain ALL microtasks (Promise.then, queueMicrotask, MutationObserver)
  3. Execute ONE macrotask (setTimeout, setInterval)
  4. Repeat from 2
  This is the spec model. Document any deviation.

**Gate:**
- Event ordering test passes
- RAF fires after microtasks

## Framework Detection (TELEMETRY ONLY)

Not for logic. For debugging and metrics.

```rust
pub enum FrameworkHint {
    React { variant: ReactVariant },  // Next.js Pages/App, Remix, CRA, Vite
    Vue { variant: VueVariant },      // Nuxt 2/3, Vite
    Svelte,                           // SvelteKit
    Angular,
    Astro,
    Unknown,
}
```

Logged in traces. Used to filter test results. NEVER used to branch execution logic.

## Diagnosis Protocol

When a site fails to hydrate:
1. Run with `NEORENDER_TRACE=1`
2. Check framework hint (telemetry)
3. Check JS errors → which runtime gap?
4. Map error to T1-T5
5. Fix the gap (generic, not per-site)
6. Verify site works
7. Verify no regressions on other sites

## Phases

### Phase 1: T1 + T2 (module + script order)
These are the biggest gaps. Most hydration failures are "module failed to load" or "script ran too early".

### Phase 2: T3 + T4 (streams + DOM mutations)
Enables RSC streaming and React reconciler correctness.

### Phase 3: T5 (event loop model)
Polish — most things already work, just verify and fix ordering edge cases.

### After all phases: Re-test
- ChatGPT (Next.js App Router)
- Mercadona.es (unknown SPA)
- GitHub (Next.js-like)
- Vercel.com (Next.js showcase)
- nuxt.com (Nuxt 3)
- svelte.dev (SvelteKit)

If they hydrate → runtime is correct. If they don't → diagnose which T1-T5 gap remains.

## T0: Instrumentation (prerequisite)

Before fixing anything, add traces for:
- Module resolution: `[MODULE] resolve foo → https://cdn.example.com/foo.js`
- Script fetch: `[SCRIPT] fetch https://... → 200 (45KB, 120ms)`
- Script execute: `[EXEC] inline#3 → ok (12ms)` or `[EXEC] main.js → error: ...`
- DOMContentLoaded/load dispatch: `[EVENT] DOMContentLoaded dispatched`
- Microtask/macrotask drain: `[LOOP] microtasks: 5, macrotasks: 2`

Without this, debugging is blind.

## Known Limitation: CSSOM

CSSOM (getComputedStyle returning real values) is outside T1-T5 scope. It may block hydration on sites where JS checks `display:none` or `visibility:hidden` before activating components. Declared as residual limitation — fix requires layout engine (V3 territory).

## Gate
- ChatGPT: entry module loads + root app mounts + observable interactive marker appears (contenteditable div or textarea detected in DOM)
- Dynamic import() works in synthetic test
- Script defer/async/module order matches spec (documented approximation: all scripts parsed upfront, defer/module grouped and executed post-parse)
- MutationObserver audit: 6/6 checks pass
- Event loop ordering test passes
- No regressions on existing 8/10 sites
- All 340+ tests pass
