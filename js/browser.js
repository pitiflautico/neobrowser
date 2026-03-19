// ═══════════════════════════════════════════════════════════════
// BROWSER BRIDGE — connects DOM events to browser-level actions
// Makes linkedom behave like a real browser from the inside.
// Loaded AFTER bootstrap.js and wom.js.
// ═══════════════════════════════════════════════════════════════

// Pending navigation/submission results — Rust reads these after eval
globalThis.__neo_pending_action = null;

// ─── 1. Link click → navigation ───
// Intercept clicks on <a> elements and trigger navigation
document.addEventListener('click', function(event) {
    let target = event.target;
    // Walk up to find the closest <a>
    while (target && target.tagName !== 'A') target = target.parentNode;
    if (target && target.tagName === 'A') {
        const href = target.getAttribute('href');
        if (href && !href.startsWith('#') && !href.startsWith('javascript:')) {
            event.preventDefault();
            // Resolve relative URLs
            let fullUrl = href;
            if (!href.startsWith('http')) {
                try { fullUrl = new URL(href, location.href).href; } catch {}
            }
            globalThis.__neo_pending_action = { type: 'navigate', url: fullUrl };
        }
    }
}, true); // capture phase

// ─── 2. Form submission → HTTP request ───
// The HTML form submission algorithm (simplified but spec-aligned)
document.addEventListener('submit', function(event) {
    event.preventDefault();
    const form = event.target;
    if (!form || form.tagName !== 'FORM') return;

    const method = (form.getAttribute('method') || 'GET').toUpperCase();
    const action = form.getAttribute('action') || location.href;

    // Collect form data (the FormData algorithm)
    const data = {};
    const elements = form.querySelectorAll('input, select, textarea');
    for (const el of elements) {
        const name = el.getAttribute('name');
        if (!name) continue;
        const type = (el.getAttribute('type') || '').toLowerCase();

        // Skip unchecked checkboxes/radios
        if ((type === 'checkbox' || type === 'radio') && !el.checked) continue;
        // Skip buttons (unless they were the submitter)
        if (type === 'submit' || type === 'button' || type === 'image' || type === 'reset') continue;
        // Skip disabled
        if (el.disabled) continue;
        // Skip file inputs (can't handle in headless)
        if (type === 'file') continue;

        data[name] = el.value || '';
    }

    // Resolve action URL
    let fullAction = action;
    if (!action.startsWith('http')) {
        try { fullAction = new URL(action, location.href).href; } catch {}
    }

    globalThis.__neo_pending_action = {
        type: 'submit',
        url: fullAction,
        method: method,
        data: data,
        enctype: form.getAttribute('enctype') || 'application/x-www-form-urlencoded',
    };
}, true);

// ─── 3. History navigation ───
// Intercept pushState/replaceState to track URL changes
const _origPushState = history.pushState?.bind(history);
const _origReplaceState = history.replaceState?.bind(history);

if (_origPushState) {
    history.pushState = function(state, title, url) {
        _origPushState(state, title, url);
        if (url) {
            try {
                const newUrl = new URL(url, location.href).href;
                location.href = newUrl;
                location.pathname = new URL(newUrl).pathname;
                // Don't trigger full navigation — SPA handles it internally
            } catch {}
        }
    };
}
if (_origReplaceState) {
    history.replaceState = function(state, title, url) {
        _origReplaceState(state, title, url);
        if (url) {
            try {
                const newUrl = new URL(url, location.href).href;
                location.href = newUrl;
                location.pathname = new URL(newUrl).pathname;
            } catch {}
        }
    };
}

// ─── 4. Interaction API for Rust ───
// These functions are called from Rust via eval

globalThis.__neo_click = function(target) {
    globalThis.__neo_pending_action = null;

    // Find element by CSS selector or text content
    let el = null;
    try { el = document.querySelector(target); } catch {}
    if (!el) {
        // Try finding by text content (case-insensitive)
        const all = document.querySelectorAll('a, button, input[type="submit"], [role="button"], [onclick]');
        for (const candidate of all) {
            const text = (candidate.textContent || candidate.value || '').trim();
            if (text.toLowerCase().includes(target.toLowerCase())) {
                el = candidate;
                break;
            }
        }
    }
    // Also try by aria-label, title, placeholder
    if (!el) {
        try {
            el = document.querySelector('[aria-label*="' + target + '" i], [title*="' + target + '" i], [placeholder*="' + target + '" i]');
        } catch {}
    }

    if (!el) return JSON.stringify({ ok: false, error: 'Element not found: ' + target });

    // Dispatch the full mouse event sequence
    const rect = { x: 0, y: 0 }; // no layout, but events need coordinates
    const opts = { bubbles: true, cancelable: true, clientX: rect.x, clientY: rect.y };

    try { el.focus?.(); } catch {}
    el.dispatchEvent(new MouseEvent('pointerdown', opts));
    el.dispatchEvent(new MouseEvent('mousedown', opts));
    el.dispatchEvent(new MouseEvent('pointerup', opts));
    el.dispatchEvent(new MouseEvent('mouseup', opts));
    el.dispatchEvent(new MouseEvent('click', opts));

    // For <a> and form submits, the event listeners above set __neo_pending_action
    // For other elements, try calling .click() directly
    if (!globalThis.__neo_pending_action) {
        try { el.click?.(); } catch {}
    }

    const action = globalThis.__neo_pending_action;
    globalThis.__neo_pending_action = null;

    if (action) {
        return JSON.stringify({ ok: true, action: action });
    }
    return JSON.stringify({ ok: true, clicked: el.tagName, text: (el.textContent||'').trim().slice(0, 100) });
};

globalThis.__neo_type = function(target, text) {
    let el = null;
    try { el = document.querySelector(target); } catch {}
    if (!el) {
        try {
            el = document.querySelector('[name="' + target + '"], [placeholder*="' + target + '" i], [aria-label*="' + target + '" i], #' + target);
        } catch {}
    }
    if (!el) {
        // Search by label text
        for (const label of document.querySelectorAll('label')) {
            if (label.textContent?.toLowerCase().includes(target.toLowerCase())) {
                const forId = label.getAttribute('for');
                if (forId) el = document.getElementById(forId);
                else el = label.querySelector('input, textarea, select');
                if (el) break;
            }
        }
    }

    if (!el) return JSON.stringify({ ok: false, error: 'Input not found: ' + target });

    try { el.focus?.(); } catch {}
    el.value = text;
    el.setAttribute('value', text);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));

    return JSON.stringify({ ok: true, typed: text.length, element: el.tagName, name: el.name || el.id || '' });
};

globalThis.__neo_submit = function(target) {
    globalThis.__neo_pending_action = null;

    let form = null;
    if (target) {
        try { form = document.querySelector(target); } catch {}
    }
    if (!form) form = document.querySelector('form');
    if (!form) return JSON.stringify({ ok: false, error: 'No form found' });

    // Dispatch submit event (listeners above will collect data)
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    const action = globalThis.__neo_pending_action;
    globalThis.__neo_pending_action = null;

    if (action) return JSON.stringify({ ok: true, action: action });
    return JSON.stringify({ ok: false, error: 'Submit event not captured' });
};

globalThis.__neo_select = function(target, value) {
    let el = null;
    try { el = document.querySelector(target); } catch {}
    if (!el) {
        try { el = document.querySelector('select[name="' + target + '"]'); } catch {}
    }
    if (!el) return JSON.stringify({ ok: false, error: 'Select not found: ' + target });

    el.value = value;
    el.dispatchEvent(new Event('change', { bubbles: true }));

    return JSON.stringify({ ok: true, selected: value });
};
