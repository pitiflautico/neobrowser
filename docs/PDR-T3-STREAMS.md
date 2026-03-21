# PDR: T3 — ReadableStream Real Implementation

## Problem
ChatGPT loads with 0 errors and 28 WOM nodes (SSR shell). But the React app doesn't hydrate — we see the server-rendered HTML but React never "activates" it. The page stays at 28 nodes instead of hundreds.

Root cause hypothesis: ChatGPT uses React Server Components (RSC) which stream data via ReadableStream. Our current implementation has `pipeThrough()` returning `self` (no-op) and no real stream consumption. RSC flight data arrives as chunks through streams — if streams don't work, React never receives the component tree.

## Current State

### What we have
- `ReadableStream` class exists (from linkedom or V8 built-in)
- `pipeThrough()` returns self (V1 hack to prevent deadlock)
- `fetch()` returns response body as string via `op_fetch`
- `Response.body` is NOT a ReadableStream — it's the raw string

### What RSC needs
1. `fetch()` returns Response where `.body` is a ReadableStream
2. ReadableStream has `.getReader()` returning `{ read() → {value, done} }`
3. `pipeThrough(TransformStream)` actually transforms chunks
4. TextDecoderStream works (byte chunks → string chunks)
5. React's flight client reads chunks incrementally to build component tree

### Evidence from ChatGPT trace
```
[MODULE] load manifest-1030f4b8.js -> ok (381KB, dynamic)
[MODULE] load 93527649-lw2de9vmoq9rz82h.js -> ok (dynamic)
```
Modules load but the app doesn't mount. The entry module likely calls `fetch('/api/...').then(r => r.body.getReader())` to start RSC streaming — this fails silently because Response.body isn't a stream.

## What to Implement

### S1: Response.body as ReadableStream

When `op_fetch` returns a response, wrap it so `response.body` is a ReadableStream that yields the body content as a single chunk.

In `js/bootstrap.js` or `js/browser_shim.js`, patch the global `fetch`:

```javascript
const _origFetch = globalThis.fetch;
globalThis.fetch = async function(url, opts) {
    const response = await _origFetch(url, opts);
    // response is currently {status, body, headers} from op_fetch
    // Wrap body as ReadableStream
    const bodyText = response.body || '';
    const encoder = new TextEncoder();
    const bodyBytes = encoder.encode(bodyText);

    const stream = new ReadableStream({
        start(controller) {
            // Yield entire body as one chunk (no real streaming, but API-compatible)
            controller.enqueue(bodyBytes);
            controller.close();
        }
    });

    return {
        ok: response.status >= 200 && response.status < 300,
        status: response.status,
        statusText: response.statusText || '',
        headers: new Headers(response.headers || {}),
        url: response.url || url,
        redirected: false,
        body: stream,
        bodyUsed: false,
        // Standard Response methods
        async text() { return bodyText; },
        async json() { return JSON.parse(bodyText); },
        async arrayBuffer() { return bodyBytes.buffer; },
        async blob() { return new Blob([bodyBytes]); },
        clone() { return this; },
    };
};
```

### S2: ReadableStream implementation

Verify or implement ReadableStream with:
- `new ReadableStream({ start(controller), pull(controller), cancel() })`
- `controller.enqueue(chunk)` — push data
- `controller.close()` — signal end
- `controller.error(e)` — signal error
- `stream.getReader()` → `{ read() → Promise<{value, done}>, releaseLock() }`
- `stream.pipeThrough(transformStream)` — REAL pipe, not no-op
- `stream.pipeTo(writableStream)` — pipe to sink
- `stream.tee()` — split into two streams
- `stream.locked` — whether a reader is active

If V8/deno_core provides ReadableStream natively → use it.
If linkedom provides it → verify correctness.
If neither → implement minimal version in JS.

Check what's available:
```javascript
typeof ReadableStream // 'function' or 'undefined'?
typeof TransformStream // ?
typeof WritableStream // ?
```

### S3: TransformStream

RSC uses `pipeThrough(new TextDecoderStream())` and custom transform streams.

```javascript
new TransformStream({
    transform(chunk, controller) {
        // Process chunk and enqueue result
        controller.enqueue(processedChunk);
    }
})
```

Needs:
- `new TransformStream({ transform, flush })`
- `.readable` — ReadableStream (output side)
- `.writable` — WritableStream (input side)
- `pipeThrough(ts)` reads from source, writes to ts.writable, returns ts.readable

### S4: TextDecoderStream / TextEncoderStream

```javascript
// Decode bytes → string
const decoderStream = new TextDecoderStream('utf-8');
// Encode string → bytes
const encoderStream = new TextEncoderStream();
```

These are TransformStreams with fixed transform functions. Implement as wrappers.

## Constraint: No Real Streaming

We can't do real incremental streaming because `op_fetch` is synchronous — it returns the complete response body at once. That's fine. The API contract is what matters:

- `response.body` IS a ReadableStream ✅
- `.getReader().read()` returns chunks (even if it's one big chunk) ✅
- `.pipeThrough()` transforms through a TransformStream ✅
- Backpressure: ignored (buffer everything) — acceptable for MVP

## What This Unblocks

If Response.body is a real ReadableStream:
1. RSC flight client can call `response.body.getReader()` ✅
2. Flight client reads chunks with `reader.read()` ✅
3. React builds component tree from flight data ✅
4. App hydrates → interactive elements appear ✅

This is the theory. It may not be the ONLY thing blocking ChatGPT hydration. But it's the most likely blocker based on the trace evidence.

## Diagnosis Plan

Before implementing, verify the hypothesis:

```bash
NEORENDER_TRACE=1 timeout 30 target/release/neorender see "https://chatgpt.com" 2>&1 | grep -i "stream\|ReadableStream\|getReader\|pipeThrough\|flight\|rsc"
```

Also run:
```javascript
eval typeof ReadableStream
eval typeof TransformStream
eval typeof Response
eval new ReadableStream({start(c){c.enqueue('test');c.close()}}).getReader ? 'has getReader' : 'no getReader'
```

If ReadableStream exists and has getReader → the issue may be elsewhere.
If ReadableStream is missing or broken → S1-S4 is the fix.

## Implementation Location

- `js/bootstrap.js` — fetch wrapper (S1), stream polyfills if needed (S2-S4)
- `crates/neo-runtime/src/ops.rs` — no changes unless op_fetch needs to return headers differently
- Do NOT touch Rust module loader or script executor

## Phases

### Phase 1: Diagnosis
- Check what stream APIs exist in our V8 context
- Identify exact point of failure in ChatGPT hydration

### Phase 2: Response.body as ReadableStream (S1)
- Patch fetch to return proper Response with body stream
- Verify: `fetch(url).then(r => r.body.getReader().read())` works

### Phase 3: Stream correctness (S2-S4)
- ReadableStream (if native is broken or missing)
- TransformStream + pipeThrough (REAL, not no-op)
- TextDecoderStream / TextEncoderStream

### Phase 4: ChatGPT re-test
- Run with traces
- Check if React mounts
- Count WOM nodes (target: >100, currently 28)

## Gate
- `fetch(url).then(r => r.body.getReader().read())` returns {value: Uint8Array, done: false}
- `pipeThrough(new TransformStream({transform(c,ctrl){ctrl.enqueue(c)}}))` works (not no-op)
- ChatGPT: WOM nodes > 50 (up from 28) OR React root mount detected
- No regressions on top 8 sites
- All existing tests pass

## Risk
This may NOT be the only blocker. ChatGPT could also fail due to:
- Missing Web Crypto APIs (subtle.digest for integrity checks)
- Missing `structuredClone` (used by React internals)
- CSS-dependent initialization (checking computed styles)
- Service Worker registration (not supported)

If streams don't fix hydration, the trace system (T0) will show the next error to chase.
