# PDR: T3 — Streams + Response Model

## Hypothesis (NOT proven)
ChatGPT loads modules but doesn't mount. ONE possible cause: Response.body isn't a ReadableStream. But could also be: missing Web Crypto, structuredClone issues, CSS checks, or other runtime gaps. This PDR addresses the streams gap regardless — it's needed for general web compat.

## Classification (honest)
- Response.body as ReadableStream: **COMPAT** (buffered, not real streaming)
- ReadableStream getReader/read: **COMPAT** (single-chunk delivery)
- pipeThrough/TransformStream: **COMPAT** (synchronous transform, no backpressure)
- TextDecoderStream: **COMPAT**
- Response consumption model: **SEMANTIC** (bodyUsed, one-shot, clone)

NOT real streaming. API-compatible buffered delivery.

## Phase 0: Diagnosis (BEFORE implementing)

### Required instrumentation
Add to fetch wrapper and stream polyfills (log only when NEORENDER_TRACE=1):

```javascript
// In fetch wrapper:
neo_trace('[FETCH] ' + method + ' ' + url);

// In Response.body getter:
neo_trace('[FETCH] response.body accessed for ' + this._url);

// In ReadableStream.getReader():
neo_trace('[STREAM] getReader() called');

// In reader.read():
neo_trace('[STREAM] read() called, remaining: ' + this._remaining + ' bytes');

// In pipeThrough():
neo_trace('[STREAM] pipeThrough() called with ' + transform.constructor.name);

// In TextDecoderStream constructor:
neo_trace('[STREAM] TextDecoderStream created, encoding=' + encoding);
```

### Run diagnosis FIRST
```bash
NEORENDER_TRACE=1 timeout 30 target/release/neorender see "https://chatgpt.com" 2>&1 | grep -E "\[FETCH\]|\[STREAM\]"
```

This tells us:
- Does any code call `response.body`? If no → streams aren't the blocker
- Does any code call `getReader()`? If no → something else fails first
- Does `pipeThrough()` get called? If no → TransformStream not needed yet

Only implement what the diagnosis shows is actually called.

## Response Consumption Model (SEMANTIC)

### Internal structure
```javascript
class NeoResponse {
    constructor(body, init) {
        this._body = body;          // Uint8Array (buffered, immutable)
        this._bodyText = null;      // lazy decoded text cache
        this._bodyUsed = false;     // one-shot flag
        this._url = init.url || '';
        this.status = init.status || 200;
        this.statusText = init.statusText || '';
        this.ok = this.status >= 200 && this.status < 300;
        this.headers = new Headers(init.headers || {});
        this.redirected = init.redirected || false;
        this.type = 'basic';
    }
}
```

### Body consumption rules (Fetch spec)
1. **One-shot**: calling any consumption method (text, json, arrayBuffer, blob, body.getReader) sets `bodyUsed = true`
2. **Double consumption throws**: if `bodyUsed` is true, throw `TypeError: body already consumed`
3. **body getter**: returns ReadableStream wrapping `_body`. Accessing body does NOT set bodyUsed (only reading from the stream does)
4. **text()**: decodes `_body` as UTF-8, sets bodyUsed
5. **json()**: calls text() then JSON.parse, sets bodyUsed
6. **arrayBuffer()**: returns `_body.buffer` copy, sets bodyUsed
7. **blob()**: returns `new Blob([_body])`, sets bodyUsed
8. **clone()**: REAL clone — creates new NeoResponse with same `_body` buffer. Both can be independently consumed. Throws if bodyUsed.
9. **body.getReader()**: locks stream, sets bodyUsed on first read

### instanceof compatibility
```javascript
// Option A: Patch global Response
globalThis.Response = NeoResponse;
// Then `new Response(body, init)` and `response instanceof Response` work

// Option B: If Response already exists (from V8/deno_core), extend it
// Check first: typeof Response
```

Decision: check if deno_core provides Response. If yes, wrap it. If no, provide NeoResponse as global Response.

## ReadableStream (COMPAT — buffered single-chunk)

### What we implement
```javascript
class NeoReadableStream {
    constructor(underlyingSource) {
        this._chunks = [];
        this._closed = false;
        this._errored = false;
        this._reader = null;

        if (underlyingSource && underlyingSource.start) {
            const controller = {
                enqueue: (chunk) => this._chunks.push(chunk),
                close: () => { this._closed = true; },
                error: (e) => { this._errored = true; this._error = e; },
            };
            underlyingSource.start(controller);
        }
    }

    get locked() { return this._reader !== null; }

    getReader() {
        if (this._reader) throw new TypeError('already locked');
        this._reader = new NeoReader(this);
        return this._reader;
    }

    pipeThrough(transform, options) {
        // REAL pipe: read all chunks, write through transform, return readable
        const reader = this.getReader();
        const writer = transform.writable.getWriter();
        // Synchronous: process all buffered chunks immediately
        (async () => {
            while (true) {
                const { value, done } = await reader.read();
                if (done) { await writer.close(); break; }
                await writer.write(value);
            }
        })();
        return transform.readable;
    }

    pipeTo(dest, options) {
        const reader = this.getReader();
        const writer = dest.getWriter();
        return (async () => {
            while (true) {
                const { value, done } = await reader.read();
                if (done) { await writer.close(); break; }
                await writer.write(value);
            }
        })();
    }

    tee() {
        const chunks = [...this._chunks];
        const closed = this._closed;
        return [
            new NeoReadableStream({ start(c) { chunks.forEach(ch => c.enqueue(ch)); if (closed) c.close(); } }),
            new NeoReadableStream({ start(c) { chunks.forEach(ch => c.enqueue(ch)); if (closed) c.close(); } }),
        ];
    }
}

class NeoReader {
    constructor(stream) {
        this._stream = stream;
        this._index = 0;
    }
    async read() {
        if (this._index < this._stream._chunks.length) {
            return { value: this._stream._chunks[this._index++], done: false };
        }
        if (this._stream._closed) return { value: undefined, done: true };
        if (this._stream._errored) throw this._stream._error;
        return { value: undefined, done: true };
    }
    releaseLock() { this._stream._reader = null; }
    get closed() { return Promise.resolve(this._stream._closed); }
    cancel() { this._stream._reader = null; return Promise.resolve(); }
}
```

### Scope constraint: fully-buffered at construction time ONLY
All chunks must be enqueued during `start()`. No pull-based or async filling.
If a stream has no chunks and isn't closed, `read()` returns `{done: true}` — this is technically wrong per spec but acceptable because we ONLY create fully-buffered streams.

### What we DON'T implement
- Pull-based reading (pull callback) — all data is buffered upfront
- Backpressure signaling — ignored
- BYOB readers — not needed
- Async iteration protocol — nice to have, not MVP
- `tee()` — EXCLUDED from MVP unless diagnosis traces show it's called. Current impl would only work on unconsumed buffered streams, too fragile for general use.

## TransformStream (COMPAT)

```javascript
class NeoTransformStream {
    constructor(transformer) {
        this._transformer = transformer || {};
        this._outputChunks = [];
        this._outputClosed = false;

        const self = this;
        this.writable = {
            getWriter() {
                return {
                    async write(chunk) {
                        if (self._transformer.transform) {
                            const ctrl = {
                                enqueue(c) { self._outputChunks.push(c); },
                                error(e) { throw e; },
                                terminate() { self._outputClosed = true; },
                            };
                            await self._transformer.transform(chunk, ctrl);
                        } else {
                            self._outputChunks.push(chunk); // passthrough
                        }
                    },
                    async close() {
                        if (self._transformer.flush) {
                            const ctrl = { enqueue(c) { self._outputChunks.push(c); } };
                            await self._transformer.flush(ctrl);
                        }
                        self._outputClosed = true;
                    },
                    releaseLock() {},
                    get closed() { return Promise.resolve(); },
                };
            }
        };

        this.readable = new NeoReadableStream({
            start(controller) {
                // Chunks populated via writable side
                // Link: when outputClosed, close this stream
                // Use polling or direct push
            }
        });
        // HACK: directly wire output chunks to readable
        this.readable._chunks = this._outputChunks;
        this.readable._closedGetter = () => self._outputClosed;
    }
}
```

## TextDecoderStream / TextEncoderStream (COMPAT)

```javascript
class TextDecoderStream extends NeoTransformStream {
    constructor(encoding = 'utf-8') {
        const decoder = new TextDecoder(encoding);
        super({
            transform(chunk, controller) {
                controller.enqueue(decoder.decode(chunk, { stream: true }));
            },
            flush(controller) {
                const final = decoder.decode();
                if (final) controller.enqueue(final);
            }
        });
    }
}

class TextEncoderStream extends NeoTransformStream {
    constructor() {
        const encoder = new TextEncoder();
        super({
            transform(chunk, controller) {
                controller.enqueue(encoder.encode(chunk));
            }
        });
    }
}
```

## Phases

### Phase 0: Diagnosis
- Add stream instrumentation
- Run against ChatGPT
- Determine what's actually called
- Only implement what's needed

### Phase 1: Response model (S1)
- NeoResponse with correct bodyUsed, clone, one-shot semantics
- Patch fetch to return NeoResponse
- Gate: `response.bodyUsed` correctly tracks, `clone()` works, double-consume throws

### Phase 2: ReadableStream (S2)
- Buffered implementation with getReader/read
- Gate: `fetch(url).then(r => r.body.getReader().read())` returns {value, done}

### Phase 3: TransformStream + pipeThrough (S3-S4)
- Only if diagnosis shows pipeThrough is actually called
- Gate: `pipeThrough(new TextDecoderStream())` produces text chunks

### Phase 4: Re-test ChatGPT
- Run with traces
- Compare WOM nodes before/after
- Check for new interactive markers

## Gate
### Phase 2 gate (detailed)
- `getReader()` does NOT set bodyUsed (only reading does)
- First `read()` returns `{value: Uint8Array, done: false}`
- Second `read()` returns `{value: undefined, done: true}`
- `text()` after `read()` throws TypeError (body already consumed)
- `clone()` before consumption → both clones independently consumable
- `clone()` after consumption → throws TypeError

### Final gate
- Response bodyUsed/clone semantics correct (assertions above)
- `fetch().then(r => r.body.getReader().read())` works
- `pipeThrough(TransformStream)` transforms chunks (if diagnosis shows it's needed)
- ChatGPT: new interactive marker appears (intermediate: different error = progression, but NOT tier exit)
- No regressions on top 8 sites
- All existing tests pass

### TransformStream caveat
The readable↔writable wiring (`_chunks` shared reference, `_closedGetter`) is an AD-HOC BRIDGE, not clean design. Acceptable for MVP. Must be replaced if stream usage grows beyond basic pipeThrough.
