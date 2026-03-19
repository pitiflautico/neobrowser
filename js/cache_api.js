// Cache API — stub (Service Worker caching)
if (!globalThis.caches) {
    class Cache {
        constructor() { this._entries = new Map(); }
        async match(request) { return this._entries.get(typeof request === 'string' ? request : request.url); }
        async put(request, response) { this._entries.set(typeof request === 'string' ? request : request.url, response); }
        async delete(request) { return this._entries.delete(typeof request === 'string' ? request : request.url); }
        async keys() { return [...this._entries.keys()]; }
        async matchAll() { return [...this._entries.values()]; }
    }

    const cacheStorage = new Map();
    globalThis.caches = {
        async open(name) { if (!cacheStorage.has(name)) cacheStorage.set(name, new Cache()); return cacheStorage.get(name); },
        async match(request) { for (const c of cacheStorage.values()) { const r = await c.match(request); if (r) return r; } },
        async has(name) { return cacheStorage.has(name); },
        async delete(name) { return cacheStorage.delete(name); },
        async keys() { return [...cacheStorage.keys()]; },
    };
}
