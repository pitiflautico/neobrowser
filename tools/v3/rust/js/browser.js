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

// ─── General element finder — resolves targets by multiple strategies ───
// Returns the DOM element or null. Used by __neo_click, __neo_find, etc.
function __neo_resolve(target) {
    let el = null;
    // 1. CSS selector
    try { el = document.querySelector(target); } catch {}
    if (el) return el;

    const targetLower = target.toLowerCase();
    const escaped = target.replace(/"/g, '\\"');

    // 2. Text content match (case-insensitive) — clickable elements first, then any
    const clickable = document.querySelectorAll('a, button, input[type="submit"], [role="button"], [role="link"], [role="tab"], [role="menuitem"], [onclick], summary');
    let partialMatch = null;
    for (const candidate of clickable) {
        const text = (candidate.textContent || candidate.value || '').trim();
        const textLower = text.toLowerCase();
        if (textLower === targetLower) return candidate;
        if (!partialMatch && textLower.includes(targetLower) && text.length < target.length * 5) {
            partialMatch = candidate;
        }
    }
    if (partialMatch) return partialMatch;

    // 3. aria-label match (case-insensitive)
    try { el = document.querySelector('[aria-label="' + escaped + '" i]'); } catch {}
    if (!el) try { el = document.querySelector('[aria-label*="' + escaped + '" i]'); } catch {}
    if (el) return el;

    // 4. placeholder match
    try { el = document.querySelector('[placeholder="' + escaped + '" i]'); } catch {}
    if (!el) try { el = document.querySelector('[placeholder*="' + escaped + '" i]'); } catch {}
    if (el) return el;

    // 5. name attribute
    try { el = document.querySelector('[name="' + escaped + '"]'); } catch {}
    if (el) return el;

    // 6. title attribute
    try { el = document.querySelector('[title="' + escaped + '" i]'); } catch {}
    if (!el) try { el = document.querySelector('[title*="' + escaped + '" i]'); } catch {}
    if (el) return el;

    // 7. data-testid
    try { el = document.querySelector('[data-testid="' + escaped + '"]'); } catch {}
    if (el) return el;

    // 8. Broader text search across ALL elements (expensive — last resort)
    const allEls = document.querySelectorAll('*');
    for (const candidate of allEls) {
        if (candidate.childNodes.length > 0 && candidate.children.length === 0) {
            // Leaf text node
            const text = (candidate.textContent || '').trim().toLowerCase();
            if (text === targetLower) return candidate;
        }
    }

    return null;
}

// Build a CSS selector path for an element (for returning to caller)
function __neo_selector_path(el) {
    if (!el || !el.tagName) return null;
    if (el.id) return '#' + el.id;
    const tag = el.tagName.toLowerCase();
    const cls = el.className ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : '';
    const name = el.getAttribute?.('name') ? '[name="' + el.getAttribute('name') + '"]' : '';
    return tag + cls + name;
}

// ─── __neo_find(target) — returns element info + selector path ───
globalThis.__neo_find = function(target) {
    const el = __neo_resolve(target);
    if (!el) return JSON.stringify({ ok: false, error: 'Element not found: ' + target });
    return JSON.stringify({
        ok: true,
        tag: el.tagName,
        text: (el.textContent || '').trim().slice(0, 200),
        selector: __neo_selector_path(el),
        attrs: {
            id: el.id || null,
            name: el.getAttribute?.('name') || null,
            type: el.getAttribute?.('type') || null,
            href: el.getAttribute?.('href') || null,
            role: el.getAttribute?.('role') || null,
            'aria-label': el.getAttribute?.('aria-label') || null,
            placeholder: el.getAttribute?.('placeholder') || null,
            'data-testid': el.getAttribute?.('data-testid') || null,
        },
    });
};

globalThis.__neo_click = function(target) {
    globalThis.__neo_pending_action = null;

    const el = __neo_resolve(target);
    if (!el) return JSON.stringify({ ok: false, error: 'Element not found: ' + target });

    // Scroll into view if available (linkedom stub returns immediately)
    try { if (el.scrollIntoView) el.scrollIntoView({ block: 'center' }); } catch {}

    // Full mouse event sequence (matches real browser order)
    const opts = { bubbles: true, cancelable: true, button: 0, clientX: 0, clientY: 0 };

    el.dispatchEvent(new MouseEvent('mouseenter', { ...opts, bubbles: false }));
    el.dispatchEvent(new MouseEvent('mouseover', opts));
    el.dispatchEvent(new MouseEvent('pointerdown', opts));
    el.dispatchEvent(new MouseEvent('mousedown', opts));
    try { el.focus?.(); } catch {}
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
    return JSON.stringify({
        ok: true,
        clicked: el.tagName,
        text: (el.textContent||'').trim().slice(0, 100),
        selector: __neo_selector_path(el),
        href: el.getAttribute?.('href') || null,
    });
};

globalThis.__neo_type = function(target, text, clear) {
    let el = __neo_find_input(target);
    // Fallback: try general resolver for contenteditable elements
    if (!el) {
        const resolved = __neo_resolve(target);
        if (resolved && (resolved.getAttribute?.('contenteditable') === 'true' || resolved.isContentEditable)) {
            el = resolved;
        }
    }

    if (!el) return JSON.stringify({ ok: false, error: 'Input not found: ' + target });

    const isContentEditable = el.getAttribute?.('contenteditable') === 'true' || el.isContentEditable;

    try { el.focus?.(); } catch {}
    el.dispatchEvent(new FocusEvent('focus', { bubbles: false }));
    el.dispatchEvent(new FocusEvent('focusin', { bubbles: true }));

    // Clear existing value if requested
    if (clear) {
        if (isContentEditable) {
            el.textContent = '';
        } else {
            el.value = '';
            el.setAttribute('value', '');
        }
        el.dispatchEvent(new Event('input', { bubbles: true }));
    }

    // Char-by-char typing with keyboard events (triggers React/Vue watchers)
    for (const char of text) {
        const keyOpts = { key: char, code: 'Key' + char.toUpperCase(), bubbles: true, cancelable: true };
        el.dispatchEvent(new KeyboardEvent('keydown', keyOpts));
        el.dispatchEvent(new KeyboardEvent('keypress', keyOpts));

        if (isContentEditable) {
            el.textContent = (el.textContent || '') + char;
        } else {
            el.value = (el.value || '') + char;
        }
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new KeyboardEvent('keyup', keyOpts));
    }

    // Final change event (fires on blur in real browsers, but frameworks often listen for it)
    el.dispatchEvent(new Event('change', { bubbles: true }));

    // Also sync the attribute for form submission
    if (!isContentEditable && el.value !== undefined) {
        el.setAttribute('value', el.value);
    }

    return JSON.stringify({
        ok: true,
        typed: text.length,
        value: isContentEditable ? el.textContent : el.value,
        element: el.tagName,
        name: el.name || el.id || '',
        selector: __neo_selector_path(el),
    });
};

globalThis.__neo_submit = function(target) {
    globalThis.__neo_pending_action = null;

    let form = null;
    if (target) {
        try { form = document.querySelector(target); } catch {}
    }
    if (!form) form = document.querySelector('form');
    if (!form) return JSON.stringify({ ok: false, error: 'No form found' });

    // ─── Detect SPA protocol FIRST ───
    const protocol = __neo_detect_form_protocol(form);

    // SPA protocols: handle natively via HTTP (don't try DOM submit — Alpine/Livewire JS won't run in linkedom)
    if (protocol.type === 'livewire') return __neo_submit_livewire(form, protocol);
    if (protocol.type === 'htmx')     return __neo_submit_htmx(form, protocol);
    if (protocol.type === 'inertia')  return __neo_submit_inertia(form, protocol);
    if (protocol.type === 'turbo')    return __neo_submit_turbo(form, protocol);

    // Standard forms: try native submit event (works if JS listeners are running)
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    const action = globalThis.__neo_pending_action;
    globalThis.__neo_pending_action = null;
    if (action) return JSON.stringify({ ok: true, protocol: 'native', action: action });

    // ─── Standard HTML fallback ───
    return __neo_submit_standard(form);
};

// ─── Protocol detection ───
function __neo_detect_form_protocol(form) {
    // Livewire v3: wire:submit on form
    const wireSubmit = form.getAttribute('wire:submit')
        || form.getAttribute('wire:submit.prevent');
    if (wireSubmit) {
        // Find the Livewire component element (closest ancestor with wire:snapshot)
        let comp = form.closest('[wire\\:snapshot]') || form.querySelector('[wire\\:snapshot]');
        if (!comp) {
            // Search all elements with wire:snapshot — the form may be inside
            comp = document.querySelector('[wire\\:snapshot]');
        }
        return {
            type: 'livewire',
            method: wireSubmit.replace('.prevent', ''),
            snapshot: comp ? comp.getAttribute('wire:snapshot') : null,
        };
    }

    // HTMX: hx-post, hx-get, hx-put, hx-patch, hx-delete on form or submit button
    for (const verb of ['post', 'get', 'put', 'patch', 'delete']) {
        const url = form.getAttribute('hx-' + verb);
        if (url) return { type: 'htmx', httpMethod: verb.toUpperCase(), url };
        // Also check submit button
        const btn = form.querySelector('[type="submit"][hx-' + verb + ']');
        if (btn) return { type: 'htmx', httpMethod: verb.toUpperCase(), url: btn.getAttribute('hx-' + verb) };
    }

    // Inertia.js: presence of Inertia version meta tag + no action attribute
    const inertiaMeta = document.querySelector('meta[name="inertia-version"]');
    if (inertiaMeta && !form.getAttribute('action')) {
        return { type: 'inertia', version: inertiaMeta.content };
    }

    // Turbo/Hotwire: data-turbo="true" or turbo-frame parent
    if (form.getAttribute('data-turbo') === 'true' || form.closest('turbo-frame')) {
        return { type: 'turbo', frame: form.closest('turbo-frame')?.id || null };
    }

    return { type: 'standard' };
}

// ─── Collect form field values ───
function __neo_collect_fields(form) {
    const data = {};
    for (const el of form.querySelectorAll('input, textarea, select')) {
        let key = el.name
            || el.getAttribute('wire:model') || el.getAttribute('wire:model.defer')
            || el.getAttribute('wire:model.live') || el.getAttribute('wire:model.lazy')
            || el.getAttribute('x-model') || el.getAttribute('v-model')
            || el.getAttribute('data-model') || el.id || null;
        if (!key) continue;
        const type = (el.type || '').toLowerCase();
        if (type === 'checkbox') data[key] = el.checked ? 'on' : '';
        else if (type === 'radio') { if (el.checked) data[key] = el.value; }
        else if (type === 'file') continue;
        else data[key] = el.value || '';
    }
    return data;
}

function __neo_get_csrf() {
    const meta = document.querySelector('meta[name="csrf-token"]');
    return meta ? meta.content : null;
}

function __neo_resolve_url(url) {
    if (!url) return location.href;
    if (url.startsWith('http')) return url;
    const base = location.origin || '';
    return url.startsWith('/') ? base + url : base + '/' + url;
}

function __neo_do_fetch(url, method, body, headers) {
    try {
        const resultJson = ops.op_neorender_fetch(
            __neo_resolve_url(url),
            method,
            typeof body === 'string' ? body : JSON.stringify(body),
            JSON.stringify(headers)
        );
        const result = JSON.parse(resultJson);
        // NEO-11: parse error body for non-2xx responses
        if (result.status >= 400) {
            try {
                const errBody = JSON.parse(result.body);
                result.error_message = errBody.message || null;
                result.error_type = errBody.exception || null;
            } catch { /* body not JSON */ }
        }
        return result;
    } catch (e) {
        return { status: 0, body: '', error_message: e.message || String(e) };
    }
}

// ─── Livewire v3 submit ───
function __neo_submit_livewire(form, protocol) {
    if (!protocol.snapshot) {
        return JSON.stringify({ ok: false, protocol: 'livewire', error: 'No wire:snapshot found' });
    }

    const fields = __neo_collect_fields(form);
    const csrf = __neo_get_csrf();

    // Build Livewire v3 update payload
    const updates = {};
    for (const [key, value] of Object.entries(fields)) {
        updates[key] = value;
    }

    const payload = JSON.stringify({
        _token: csrf,
        components: [{
            snapshot: protocol.snapshot,
            updates: updates,
            calls: [{ method: protocol.method, params: [] }]
        }]
    });

    const result = __neo_do_fetch('/livewire/update', 'POST', payload, {
        'Content-Type': 'application/json',
        'Accept': 'application/json',
        'X-Livewire': 'true',
    });

    // Parse Livewire response
    let redirect = null;
    let effects = {};
    let newSnapshot = null;
    if (result.status === 200 && result.body) {
        try {
            const resp = JSON.parse(result.body);
            const comp = resp.components?.[0];
            effects = comp?.effects || {};
            redirect = effects.redirect || null;
            newSnapshot = comp?.snapshot || null;
            // Update the snapshot in the DOM for next submit
            if (newSnapshot) {
                const snapshotEl = document.querySelector('[wire\\:snapshot]');
                if (snapshotEl) snapshotEl.setAttribute('wire:snapshot', newSnapshot);
            }
        } catch {}
    }

    return JSON.stringify({
        ok: result.status >= 200 && result.status < 400,
        protocol: 'livewire',
        status: result.status,
        method_called: protocol.method,
        redirect: redirect,
        effects: Object.keys(effects),
        error_message: result.error_message || null,
        action: redirect ? { url: redirect, method: 'GET' } : null,
    });
}

// ─── HTMX submit ───
function __neo_submit_htmx(form, protocol) {
    const fields = __neo_collect_fields(form);
    const csrf = __neo_get_csrf();
    if (csrf) fields['_token'] = csrf;

    // HTMX sends form-encoded by default
    const body = Object.entries(fields).map(([k,v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&');

    const headers = {
        'Content-Type': 'application/x-www-form-urlencoded',
        'HX-Request': 'true',
        'HX-Current-URL': location.href,
    };
    // hx-target, hx-trigger
    const hxTarget = form.getAttribute('hx-target');
    if (hxTarget) headers['HX-Target'] = hxTarget.replace('#', '');
    const hxTrigger = form.getAttribute('hx-trigger') || form.id;
    if (hxTrigger) headers['HX-Trigger'] = hxTrigger;

    const result = __neo_do_fetch(protocol.url, protocol.httpMethod, body, headers);

    // HTMX can return HTML to swap into a target, or HX-Redirect header
    const redirect = null; // Would need to check response headers
    return JSON.stringify({
        ok: result.status >= 200 && result.status < 400,
        protocol: 'htmx',
        status: result.status,
        redirect: redirect,
        has_html: result.body?.length > 0,
        error_message: result.error_message || null,
        action: { url: protocol.url, method: protocol.httpMethod, data: fields },
    });
}

// ─── Inertia.js submit ───
function __neo_submit_inertia(form, protocol) {
    const fields = __neo_collect_fields(form);
    const csrf = __neo_get_csrf();
    const url = form.action || form.getAttribute('action') || location.pathname;
    const method = (form.method || 'POST').toUpperCase();

    const headers = {
        'Content-Type': 'application/json',
        'Accept': 'text/html, application/xhtml+xml',
        'X-Inertia': 'true',
        'X-Inertia-Version': protocol.version || '',
        'X-Requested-With': 'XMLHttpRequest',
    };
    if (csrf) headers['X-CSRF-TOKEN'] = csrf;

    const result = __neo_do_fetch(url, method, JSON.stringify(fields), headers);

    // Inertia returns JSON with component + props, or redirect
    let redirect = null;
    if (result.status === 409) {
        // Inertia version conflict → need page reload
        redirect = location.href;
    } else if (result.body) {
        try {
            const resp = JSON.parse(result.body);
            redirect = resp.url || null;
        } catch {}
    }

    return JSON.stringify({
        ok: result.status >= 200 && result.status < 400,
        protocol: 'inertia',
        status: result.status,
        redirect: redirect,
        error_message: result.error_message || null,
        action: redirect ? { url: redirect, method: 'GET' } : { url, method, data: fields },
    });
}

// ─── Turbo/Hotwire submit ───
function __neo_submit_turbo(form, protocol) {
    const fields = __neo_collect_fields(form);
    const csrf = __neo_get_csrf();
    if (csrf) fields['_token'] = csrf;
    const url = form.action || form.getAttribute('action') || location.pathname;
    const method = (form.method || 'POST').toUpperCase();

    const body = Object.entries(fields).map(([k,v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&');

    const headers = {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Accept': 'text/vnd.turbo-stream.html, text/html, application/xhtml+xml',
        'Turbo-Frame': protocol.frame || '_top',
    };

    const result = __neo_do_fetch(url, method, body, headers);

    return JSON.stringify({
        ok: result.status >= 200 && result.status < 400,
        protocol: 'turbo',
        status: result.status,
        frame: protocol.frame,
        has_stream: result.body?.includes('turbo-stream'),
        error_message: result.error_message || null,
        action: { url, method, data: fields },
    });
}

// ─── Standard HTML form submit ───
function __neo_submit_standard(form) {
    const fields = __neo_collect_fields(form);
    const csrf = __neo_get_csrf();
    if (csrf && !fields['_token']) fields['_token'] = csrf;
    const url = form.action || form.getAttribute('action') || location.pathname;
    const method = (form.method || form.getAttribute('method') || 'POST').toUpperCase();

    return JSON.stringify({
        ok: true,
        protocol: 'standard',
        action: { url, method, data: fields },
    });
}

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

// Shared element finder — used by fill_form and type
function __neo_find_input(target) {
    let el = null;
    // 1. CSS selector
    try { el = document.querySelector(target); if (el && !['INPUT','TEXTAREA','SELECT'].includes(el.tagName)) el = null; } catch {}
    // 2. Attribute id (handles dots: [id="data.email"])
    if (!el) { try { el = document.querySelector('[id="' + target + '"]'); if (el && !['INPUT','TEXTAREA','SELECT'].includes(el.tagName)) el = null; } catch {} }
    // 3. wire:model / x-model / v-model
    if (!el) { try { el = document.querySelector('[wire\\:model="' + target + '"], [wire\\:model\\.live="' + target + '"], [wire\\:model\\.defer="' + target + '"], [x-model="' + target + '"], [v-model="' + target + '"]'); } catch {} }
    // 4. name / placeholder / aria-label
    if (!el) { try { const c = document.querySelectorAll('input[name="' + target + '"], textarea[name="' + target + '"], select[name="' + target + '"], input[placeholder*="' + target + '" i], textarea[placeholder*="' + target + '" i], input[aria-label*="' + target + '" i], textarea[aria-label*="' + target + '" i]'); el = c[0] || null; } catch {} }
    // 5. Label text
    if (!el) {
        for (const label of document.querySelectorAll('label')) {
            if (label.textContent?.toLowerCase().includes(target.toLowerCase())) {
                const forId = label.getAttribute('for');
                if (forId) el = document.querySelector('[id="' + forId + '"]');
                if (!el) el = label.querySelector('input, textarea, select');
                if (el) break;
            }
        }
    }
    // 6. data-testid fallback
    if (!el) { try { el = document.querySelector('[data-testid="' + target + '"]'); if (el && !['INPUT','TEXTAREA','SELECT'].includes(el.tagName)) el = null; } catch {} }
    return el;
}

globalThis.__neo_fill_form = function(fieldsJson) {
    const fields = JSON.parse(fieldsJson);
    const results = [];
    for (const [target, value] of Object.entries(fields)) {
        let el = __neo_find_input(target);
        if (!el) {
            results.push({ field: target, ok: false, error: 'not found' });
            continue;
        }
        try { el.focus?.(); } catch {}
        const elType = (el.getAttribute('type') || '').toLowerCase();

        if (el.tagName === 'SELECT') {
            // Select: set value + change event
            el.value = value;
            el.dispatchEvent(new Event('change', { bubbles: true }));
        } else if (elType === 'checkbox') {
            // Checkbox: value "true"/"1"/"on"/"yes" → check, anything else → uncheck
            const shouldCheck = ['true', '1', 'on', 'yes'].includes(value.toLowerCase());
            if (el.checked !== shouldCheck) {
                el.checked = shouldCheck;
                el.dispatchEvent(new Event('input', { bubbles: true }));
                el.dispatchEvent(new Event('change', { bubbles: true }));
                // Also dispatch click for frameworks that listen on click (Alpine, React)
                el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
            }
        } else if (elType === 'radio') {
            // Radio: check this one, dispatch change
            el.checked = true;
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
        } else if (elType === 'file') {
            // File input: value is path(s), create synthetic FileList via DataTransfer
            try {
                const dt = new DataTransfer();
                const paths = value.split(',').map(p => p.trim());
                for (const path of paths) {
                    const name = path.split('/').pop() || 'file';
                    const ext = name.split('.').pop()?.toLowerCase() || '';
                    const mimeMap = { pdf: 'application/pdf', jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', gif: 'image/gif', txt: 'text/plain', json: 'application/json', csv: 'text/csv' };
                    const mime = mimeMap[ext] || 'application/octet-stream';
                    dt.items.add(new File([''], name, { type: mime }));
                }
                el.files = dt.files;
                el.dispatchEvent(new Event('input', { bubbles: true }));
                el.dispatchEvent(new Event('change', { bubbles: true }));
            } catch (e) {
                results.push({ field: target, ok: false, error: 'file_input: ' + e.message });
                continue;
            }
        } else if (elType === 'date') {
            // Date input: set value in YYYY-MM-DD format
            el.value = value;
            el.setAttribute('value', value);
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
        } else {
            // Default: text, email, password, number, tel, url, textarea, etc.
            el.value = value;
            el.setAttribute('value', value);
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
        }
        results.push({ field: target, ok: true, tag: el.tagName, type: elType || el.tagName.toLowerCase(), name: el.name || el.id || '' });
    }
    return JSON.stringify({ ok: true, filled: results.filter(r => r.ok).length, total: results.length, results: results });
};
