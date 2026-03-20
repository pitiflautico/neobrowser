# PDR: ChatGPT Hydration Performance (>60s → <10s)

## Current Bottlenecks (profiled)

| Phase | Time | Cause | Fix |
|---|---|---|---|
| Module fetch on-demand | ~5s | `4813494d` (1.9MB) not pre-fetched | Pre-fetch all modulepreload deps |
| DataDog/telemetry fetches | ~3s | Not skipped by filter | Add to skip list |
| Event loop post-scripts | 15s | Fixed timeout, even if no work | Adaptive: exit when idle |
| Stability check | 30s timeout | setInterval keeps DOM "changing" | Reduce threshold or timeout |
| setInterval ticks | ~5s | 50 ticks × 100ms timer | Reduce cap or timer delay |
| Module evaluation TLA | ~5s | Vendor Promise.allSettled chain | Already fixed via source rewrite |

**Total current: ~60s+**
**Target: <10s**

## Plan (ordered by impact)

### T1: Pre-fetch ALL module dependencies (saves ~5s)

Currently: modulepreloads are pre-fetched (5 modules). But their imports
(`4813494d`, `47edf3d1`) are fetched on-demand during module evaluation.

Fix: After pre-fetching modulepreloads in session.rs step 7, scan their
imports and pre-fetch transitively. The depth-3 scan already exists but
only scans the initial modules, not the modulepreloads.

In `session.rs` step 7 (pre-fetch ES module imports), include modulepreload
scripts in the scan input.

### T2: Skip ALL analytics/telemetry fetches (saves ~3s)

Current skip list misses: `datadoghq`, `browser-intake`, `oai/log`.

Add to `ops.rs` skip filter:
- `datadoghq`
- `browser-intake`
- `oai/log`
- `cdn.mxpnl.com` (Mixpanel)
- `sentry.io`

### T3: Adaptive post-script event loop (saves ~10s)

Current: fixed 15s `run_event_loop` after scripts.

Fix: Poll event loop in 200ms increments. Exit early when:
- No pending JS promises/microtasks
- No pending fetch ops
- DOM node count stable for 1s

```rust
// Instead of fixed 15s:
for _ in 0..75 { // max 15s
    run_event_loop(&mut self.runtime, 200).await.ok();
    let idle = eval("!globalThis.__neo_has_pending_work()");
    if idle { break; }
}
```

### T4: Reduce stability timeout (saves ~15s)

Current: 200ms polls, 15 stable checks = 3s stable threshold, 30s timeout.

For pages with modules that completed: reduce to 500ms stable, 5s timeout.

```rust
let stability_timeout = if scripts_count > 10 {
    Duration::from_secs(5) // SPA with many modules — faster cutoff
} else {
    Duration::from_secs(15) // Simple page
};
```

### T5: setInterval immediate mode (saves ~5s)

Current: setInterval ticks capped at 50, each tick does `op_neorender_timer(ms)`
which sleeps `min(ms, 100)` milliseconds.

Fix: For the first 10 ticks, use 0ms delay (immediate). After that, cap at 50ms.
Most React lifecycle timers only need 1-3 ticks.

### T6: Parallel module pre-fetch (saves ~2s)

Current: modules fetched sequentially in the import scanner.

Fix: Use `futures::future::join_all` to fetch all pending modules in parallel.

## Expected result

| Phase | Before | After |
|---|---|---|
| Module fetch | 5s | 1s (pre-fetched + parallel) |
| Telemetry | 3s | 0s (skipped) |
| Post-script loop | 15s | 1-2s (adaptive) |
| Stability | 30s | 2-3s (reduced timeout) |
| Timer ticks | 5s | 1s (immediate mode) |
| **Total** | **>60s** | **~5-8s** |

## Implementation order

1. T2 (skip telemetry) — 5 min, immediate impact
2. T1 (pre-fetch deps) — 30 min, reduces on-demand fetches to 0
3. T4 (reduce stability timeout) — 5 min
4. T3 (adaptive event loop) — 20 min
5. T5 (setInterval immediate) — 10 min
6. T6 (parallel fetch) — 15 min

## Test

After all fixes:
```bash
timeout 15 neobrowser_rs mcp << 'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"https://chatgpt.com","mode":"neorender","cookies_file":"/tmp/chatgpt-fresh.json"}}}
EOF
```
Should complete in <10s with `routeModules=true`.
