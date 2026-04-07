// Dynamic script loader — intercept script tags added to DOM.
// When JS does `document.head.appendChild(script)`, we capture the src
// and queue it for Rust to fetch + execute. This is what makes a browser a browser.

(function() {
    'use strict';

    // Queue of scripts to load — Rust polls this via __neo_pending_scripts()
    const _pendingScripts = [];
    // Track loaded scripts to avoid duplicates
    const _loadedScripts = new Set();
    // Track script callbacks
    const _scriptCallbacks = new Map();
    let _scriptIdCounter = 0;

    // Expose to Rust
    globalThis.__neo_pending_scripts = function() {
        const batch = _pendingScripts.splice(0);
        return JSON.stringify(batch);
    };

    // Called by Rust after executing a script
    globalThis.__neo_script_loaded = function(id, src) {
        _loadedScripts.add(src);
        const cb = _scriptCallbacks.get(id);
        if (cb) {
            _scriptCallbacks.delete(id);
            try {
                if (cb.onload) cb.onload();
                const el = cb.element;
                if (el) {
                    el.dispatchEvent(new Event('load'));
                }
            } catch(e) {}
        }
    };

    // Called by Rust if script fetch fails
    globalThis.__neo_script_error = function(id, src, error) {
        const cb = _scriptCallbacks.get(id);
        if (cb) {
            _scriptCallbacks.delete(id);
            try {
                if (cb.onerror) cb.onerror(new Error(error));
                const el = cb.element;
                if (el) {
                    el.dispatchEvent(new Event('error'));
                }
            } catch(e) {}
        }
    };

    // Intercept appendChild and insertBefore on Node.prototype
    const origAppendChild = Node.prototype.appendChild;
    const origInsertBefore = Node.prototype.insertBefore;

    function interceptScript(node) {
        if (!node || node.nodeType !== 1) return;
        if (node.tagName !== 'SCRIPT') return;

        const src = node.getAttribute('src') || node.src;
        if (!src) return; // Inline script — linkedom handles it
        if (_loadedScripts.has(src)) return; // Already loaded

        const id = ++_scriptIdCounter;
        const isModule = node.getAttribute('type') === 'module';
        const isAsync = node.hasAttribute('async');
        const isDefer = node.hasAttribute('defer');

        // Store callback info
        _scriptCallbacks.set(id, {
            element: node,
            onload: node.onload,
            onerror: node.onerror,
        });

        // Queue for Rust to fetch
        _pendingScripts.push({
            id: id,
            src: src,
            module: isModule,
            async: isAsync,
            defer: isDefer,
        });
    }

    Node.prototype.appendChild = function(node) {
        const result = origAppendChild.call(this, node);
        interceptScript(node);
        return result;
    };

    Node.prototype.insertBefore = function(node, ref) {
        const result = origInsertBefore.call(this, node, ref);
        interceptScript(node);
        return result;
    };

    // Also intercept document.createElement to track script elements
    const origCreateElement = document.createElement.bind(document);
    document.createElement = function(tag, options) {
        const el = origCreateElement(tag, options);
        if (tag.toLowerCase() === 'script') {
            // Watch for src being set later
            let _src = '';
            const origSetAttribute = el.setAttribute.bind(el);
            el.setAttribute = function(name, value) {
                origSetAttribute(name, value);
                if (name === 'src') _src = value;
            };
            // Define src property
            Object.defineProperty(el, 'src', {
                get() { return _src || el.getAttribute('src') || ''; },
                set(v) { _src = v; el.setAttribute('src', v); },
                configurable: true,
            });
        }
        return el;
    };

})();
