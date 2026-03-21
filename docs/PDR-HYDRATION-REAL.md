# PDR: Real Hydration — Making SPAs Mount in V8+linkedom

## Diagnosis: ChatGPT

### What works
- Cloudflare bypassed (Chrome136 coherent fingerprint) ✅
- HTML fetched (authenticated page with SSR shell) ✅
- 18 inline scripts execute without errors ✅
- DOMContentLoaded + load events dispatched ✅
- Dynamic import() chains work: manifest → chunk → chunk (1834KB loaded) ✅
- **0 JS errors during execution** ✅

### What doesn't work
- React/ReactDOM are NOT on globalThis (undefined) — they're inside ES module scope
- The app doesn't mount — no interactive React elements in DOM
- 27 WOM nodes = SSR shell only (sidebar, textarea from server HTML)

### Root cause hypothesis
Modules load and their top-level code executes. But the app initialization (React.createRoot().render() or equivalent) happens asynchronously — either:
1. **Event loop doesn't drain far enough**: module loads → schedules microtask → we stop before it runs
2. **Missing browser API**: React init calls something we don't have (IntersectionObserver, ResizeObserver, requestIdleCallback, etc.) and silently fails
3. **async/await in module top-level**: top-level await in entry module blocks in deno_core's module evaluation

## Investigation needed (BEFORE writing code)

### Test 1: Event loop drainage
After all scripts + modules execute, how many pending tasks remain?
```javascript
// Add to pipeline after settle phase:
console.log("pending timers:", __tracker.timers);
console.log("pending promises:", __tracker.promises);
console.log("pending fetches:", __tracker.fetches);
```

### Test 2: What the entry module does
Examine `inline-module#9` (the one that triggers dynamic imports).
```bash
# Extract the inline module source
curl -s -b "cookies..." "https://chatgpt.com" | grep -o '<script type="module">[^<]*</script>'
```
What does it do? Does it call import()? Does it await? Does it register a callback?

### Test 3: Module evaluation completion
Are the dynamically imported modules fully evaluated? Or do they fail mid-execution?
```javascript
// After module load in modules.rs, check if the module has exports
```

### Test 4: Silent failures
Many of our browser shims return fake data. Does React/Next.js check:
- `window.location.pathname` (we have this ✅)
- `document.cookie` (we have this ✅)
- `navigator.serviceWorker` (NOT shimmed — may throw)
- `performance.getEntriesByType('navigation')` (returns empty — may confuse)
- `window.crypto.subtle` (NOT shimmed — may throw)
- `caches` (Service Worker Cache API — NOT shimmed)
- `document.createRange` (linkedom may not support)
- `window.getSelection` (linkedom may not support)

### Test 5: Check what ChatGPT's entry module ACTUALLY does
Download and read the manifest + entry chunk:
```bash
curl -s "https://chatgpt.com/cdn/assets/manifest-1030f4b8.js" | head -20
```

## What to implement (based on investigation)

Only after the 5 tests above reveal the actual blocker. Possible fixes:

### If event loop doesn't drain enough:
- Extend settle phase: run event loop for longer (currently stops too early)
- Pump microtasks explicitly after module evaluation
- Run V8 event loop with poll_event_loop() after all modules load

### If missing browser APIs:
- Add targeted shims for what React actually calls
- navigator.serviceWorker → stub with { ready: Promise.resolve({}) }
- window.crypto.subtle → stub (or use V8's built-in if available)
- document.createRange → implement if linkedom doesn't have it
- window.getSelection → stub returning empty selection

### If top-level await blocks:
- deno_core's mod_evaluate returns a JoinFuture — we may not be awaiting it fully
- Need to pump the event loop until module evaluation completes

### If module execution errors silently:
- Wrap module evaluation with error capture
- Check deno_core's module error reporting

## Phases

### Phase 0: Investigation (NO code changes)
Run the 5 tests above. Document findings.
Outcome: know exactly which gap blocks ChatGPT hydration.

### Phase 1: Fix the identified gap
Based on Phase 0 findings. One targeted fix, not shotgun approach.

### Phase 2: Verify and generalize
- ChatGPT mounts (interactive elements appear beyond SSR shell)
- Test on Mercadona.es, Vercel.com, nuxt.com
- No regressions

## Anti-pattern: DO NOT
- Write per-framework patches (already rejected by GPT review)
- Guess and shotgun-fix multiple things at once
- Add shims without evidence they're needed
- Assume "0 errors = everything works" (silent failures are the problem)

## Gate
- Phase 0 complete: exact blocker identified and documented
- Phase 1: ChatGPT React mounts (new elements appear beyond SSR shell, OR clear documented reason why it can't)
- Mercadona.es: content loads (not just challenge page)
- All existing tests pass
