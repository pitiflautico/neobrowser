// NeoRender — Request interceptor
// Wraps globalThis.fetch to log every request transparently.
// Always active — no enable/disable needed.

globalThis.__neo_network_log = [];
const _realFetch = globalThis.fetch;
globalThis.fetch = function(input, init) {
    const url = typeof input === 'string' ? input : input?.url || '';
    const method = init?.method || 'GET';
    const startTime = Date.now();
    const entry = { method, url: url.slice(0, 200), status: 0, size: 0, duration: 0, timestamp: startTime };
    globalThis.__neo_network_log.push(entry);

    // Cap log at 500 entries to avoid unbounded memory growth
    if (globalThis.__neo_network_log.length > 500) {
        globalThis.__neo_network_log = globalThis.__neo_network_log.slice(-250);
    }

    const result = _realFetch(input, init);
    if (result && result.then) {
        result.then(resp => {
            entry.status = resp.status;
            entry.duration = Date.now() - startTime;
        }).catch(() => { entry.status = -1; entry.duration = Date.now() - startTime; });
    }
    return result;
};

// API: get log entries, optionally filtered by URL substring or method
globalThis.__neo_get_network_log = function(filter) {
    let log = globalThis.__neo_network_log;
    if (filter) {
        log = log.filter(e => e.url.includes(filter) || e.method === filter);
    }
    return JSON.stringify(log);
};

// API: clear the log
globalThis.__neo_clear_network_log = function() {
    globalThis.__neo_network_log = [];
};
