// NeoRender — Wait-for-condition helpers
// Injected into V8 runtime. Rust-side polls these for async waiting.

globalThis.__neo_wait_for = function(selector, timeout_ms) {
    // Check immediately — if found, return success with zero wait
    if (document.querySelector(selector)) {
        return JSON.stringify({found: true, waited: 0});
    }
    // Can't truly block in sync JS — return not-found so Rust can poll
    return JSON.stringify({found: false});
};

globalThis.__neo_wait_for_text = function(text, timeout_ms) {
    const body = document.body?.textContent || '';
    if (body.includes(text)) {
        return JSON.stringify({found: true});
    }
    return JSON.stringify({found: false});
};

globalThis.__neo_wait_for_stable = function() {
    // Snapshot current DOM child count for stability detection
    const count = document.body ? document.body.children.length : 0;
    return JSON.stringify({children_count: count});
};
