# PDR: Async Timer Op — The Final Piece for React Hydration

## Problem (exact)
React scheduler needs to yield between work chunks. It posts a setTimeout(fn, 0) callback. In a real browser, this is a macrotask — it yields to the event loop, lets other work run, then fires. In our V8:

- `op_timer` is SYNC → `thread::sleep(1ms)` → blocks V8 thread → watchdog can't interrupt → hangs
- `queueMicrotask` is too fast → no yielding → tight microtask loop → hangs
- `op_timer_async` panics → "no tokio reactor running" → deno_core async ops need proper setup

## What we proved
- Manual `createRoot().render()` in REPL produces ChatGPT content ("Hey, perez. Ready to dive in!")
- React fibers attach (1 detected)
- The ONLY thing blocking full hydration: `startTransition` schedules work via scheduler → scheduler uses setTimeout → setTimeout doesn't yield properly

## What Deno does
Real Deno has setTimeout as an async op. From [Deno internals](https://choubey.gitbook.io/internals-of-deno/foundations/evaluate-module):
- Timer ops are async — they return a Future
- The Future resolves via tokio::time::sleep
- deno_core's event loop polls these Futures alongside JS microtasks
- When the timer resolves, the JS callback fires

## What we need to build

### Piece 1: Async timer op in deno_core

```rust
// In ops.rs
#[op2(async)]
pub async fn op_timer_async(#[smi] ms: u32) -> () {
    let delay = std::cmp::max(ms, 0).min(10) as u64;
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    } else {
        tokio::task::yield_now().await;
    }
}
```

Problem: this panics because our V8 runtime's execute_script doesn't run inside a tokio context.

### Piece 2: Understanding the tokio reactor issue

Our `DenoRuntime` has `tokio_rt: tokio::runtime::Runtime` but:
- `execute_script` runs V8 synchronously, NOT inside `tokio_rt.block_on()`
- Async ops need to be INSIDE a tokio context to resolve
- `run_event_loop` IS inside `tokio_rt.block_on()` — that's where async ops resolve
- But our `execute_script` (which runs inline scripts) is NOT inside block_on

So: async ops registered during `execute_script` will ONLY resolve when `run_event_loop` is called later.

This is actually CORRECT behavior for setTimeout:
1. JS calls `setTimeout(fn, 0)` → triggers `op_timer_async(0)`
2. The async op is PENDING (registered but not resolved)
3. `execute_script` returns
4. `run_event_loop` is called → tokio processes the async op → resolves → callback fires

This is EXACTLY macrotask semantics! The callback doesn't fire immediately (like queueMicrotask). It fires on the next event loop tick.

### Piece 3: Why the panic occurs

The panic `no reactor running` happens because `op_timer_async` is declared as `#[op2(async)]` which means deno_core tries to spawn a tokio Future. But during `execute_script`, there's no tokio runtime context.

**However**: deno_core SHOULD handle this. When you declare an async op, deno_core is supposed to:
1. Register the Future in its pending ops queue
2. NOT execute it immediately
3. Execute it later when `run_event_loop` polls pending ops

The panic suggests our deno_core version (0.311) or our setup doesn't properly register async ops.

### Piece 4: Investigation needed

Check how Deno's own timer works:
- Source: `ext/web/timers.rs` in deno repository
- It uses `#[op2(async)]` or `#[op]` with `OpState`
- The timer is registered as a pending op and resolved by the event loop

Specifically check:
1. Does deno_core 0.311 support `#[op2(async)]`?
2. Does it need `#[op2(async)]` or `#[op2(async, lazy)]`?
3. Does the extension need `state` parameter setup?
4. Does our `new_inner()` in v8.rs create the runtime correctly for async ops?

### Piece 5: The JS bridge

Currently `setTimeout(fn, 0)` calls `queueMicrotask` which is sync. We need:

```javascript
// In bootstrap.js
globalThis.setTimeout = function(fn, ms, ...args) {
    if (!ms || ms <= 0) {
        // ASYNC: returns a Promise that resolves on next event loop tick
        ops.op_timer_async(0).then(() => {
            // Callback fires AFTER yielding to event loop (macrotask semantics)
            fn(...args);
        });
    } else {
        ops.op_timer_async(Math.min(ms, 10)).then(() => {
            fn(...args);
        });
    }
};
```

This makes ALL setTimeout callbacks fire as macrotasks (resolved by event loop, not microtask queue).

### Piece 6: Timer budget integration

With async timers, the timer budget check moves:
```javascript
ops.op_timer_async(delay).then(() => {
    if (ops.op_timer_fire()) {  // budget check
        fn(...args);
    }
});
```

### Piece 7: MessageChannel re-enable

With proper async setTimeout, MessageChannel can use setTimeout(fn, 0) safely:
```javascript
postMessage(data) {
    setTimeout(() => {  // now truly async — yields to event loop
        target.onmessage(event);
    }, 0);
}
```

React scheduler with MessageChannel + async setTimeout = proper yielding.

### Piece 8: Settle phase compatibility

`run_until_settled` already calls `run_event_loop` which processes async ops. So:
1. Script executes → calls setTimeout(fn, 0) → registers async op
2. `execute_script` returns
3. `run_until_settled` → `run_event_loop` → tokio resolves timer → callback fires
4. Callback does work → calls setTimeout again → new async op
5. Loop continues until React finishes (yields via performance.now() deadline)

This is EXACTLY how a real browser event loop works.

### Piece 9: Impact on existing sites

- Sites without React scheduler: no change (setTimeout > 0 already uses timerPromise which is sync)
- Sites with React scheduler: hydration works (scheduler yields properly)
- HN, Wikipedia, etc.: no impact (they don't use concurrent React)
- ChatGPT: full hydration (startTransition flushes correctly)

### Piece 10: What could go wrong

1. **deno_core 0.311 async op registration**: may need specific setup
2. **RefCell panics**: deno_core async ops sometimes panic on concurrent borrow — why we made everything sync originally
3. **Performance**: async ops have overhead vs sync — but for timers it's negligible
4. **Ordering**: async timer resolution order may differ from sync — shouldn't matter for setTimeout

## Implementation plan

### Step 1: Research deno_core async ops (30 min)
- Read deno_core 0.311 source for `#[op2(async)]` handling
- Check if `lazy` flag needed
- Check if runtime needs special init for async ops
- Look at Deno's own `ext/web/timers.rs`

### Step 2: Create op_timer_async correctly (1 hour)
- Handle the tokio reactor context issue
- May need to wrap in `spawn_blocking` or use deno_core's own async mechanism
- Test: `op_timer_async(0)` doesn't panic

### Step 3: Wire setTimeout to async op (30 min)
- bootstrap.js: setTimeout(fn, 0) → op_timer_async(0).then(fn)
- Keep budget checks
- Keep timer_register/timer_fire for tracking

### Step 4: Re-enable MessageChannel (15 min)
- postMessage → setTimeout(fn, 0) → async op → macrotask

### Step 5: Test ChatGPT hydration (30 min)
- Run `neorender see https://chatgpt.com`
- Check: nodes > 25? fibers > 0?
- Check: "Hey, perez" in content?

### Step 6: Test all sites regression (30 min)
- HN, Vercel, react.dev, svelte.dev, vuejs.org
- Local React PONG
- DDG form submit

## Gate
- op_timer_async(0) doesn't panic
- setTimeout(fn, 0) fires on next event loop tick (not microtask)
- ChatGPT: React fibers > 0 in pipeline output
- Local React PONG still works
- All 6+ sites still work
- No regressions in 370+ tests
