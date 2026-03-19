// Complete History API with proper event dispatch
(function() {
    const _history = [];
    let _currentIndex = -1;
    let _currentState = null;

    // Track navigations
    const historyAPI = {
        get length() { return _history.length; },
        get state() { return _currentState; },

        pushState(state, title, url) {
            // Remove forward entries
            _history.splice(_currentIndex + 1);
            _history.push({ state, title, url: url || location.href });
            _currentIndex = _history.length - 1;
            _currentState = state;
            if (url) {
                try {
                    const parsed = new URL(url, location.href);
                    location.href = parsed.href;
                    location.pathname = parsed.pathname;
                    location.search = parsed.search;
                    location.hash = parsed.hash;
                } catch {}
            }
        },

        replaceState(state, title, url) {
            if (_currentIndex >= 0) {
                _history[_currentIndex] = { state, title, url: url || location.href };
            }
            _currentState = state;
            if (url) {
                try {
                    const parsed = new URL(url, location.href);
                    location.href = parsed.href;
                    location.pathname = parsed.pathname;
                    location.search = parsed.search;
                    location.hash = parsed.hash;
                } catch {}
            }
        },

        back() {
            if (_currentIndex > 0) {
                _currentIndex--;
                const entry = _history[_currentIndex];
                _currentState = entry.state;
                _dispatchPopState(entry.state);
            }
        },

        forward() {
            if (_currentIndex < _history.length - 1) {
                _currentIndex++;
                const entry = _history[_currentIndex];
                _currentState = entry.state;
                _dispatchPopState(entry.state);
            }
        },

        go(delta) {
            const newIndex = _currentIndex + (delta || 0);
            if (newIndex >= 0 && newIndex < _history.length) {
                _currentIndex = newIndex;
                const entry = _history[_currentIndex];
                _currentState = entry.state;
                _dispatchPopState(entry.state);
            }
        }
    };

    function _dispatchPopState(state) {
        const event = new PopStateEvent('popstate', { state });
        globalThis.dispatchEvent(event);
    }

    // Override globalThis.history
    globalThis.history = historyAPI;

    // Initialize with current URL
    _history.push({ state: null, title: '', url: location.href });
    _currentIndex = 0;

    // Also handle hashchange
    let _lastHash = location.hash;
    // Check periodically (no MutationObserver for location changes)
    // SPAs usually call pushState, which we intercept above
})();

// Navigation API (newer spec, used by some modern frameworks)
if (!globalThis.navigation) {
    globalThis.navigation = {
        currentEntry: { url: location.href, key: '', id: '', index: 0, sameDocument: true },
        entries() { return [this.currentEntry]; },
        canGoBack: false,
        canGoForward: false,
        addEventListener() {},
        removeEventListener() {},
        navigate(url) { location.href = url; },
    };
}

// Intercept location assignments
const _locationProxy = new Proxy(location, {
    set(target, prop, value) {
        if (prop === 'href' || prop === 'pathname' || prop === 'search') {
            target[prop] = value;
            // Signal to Rust that navigation is needed
            globalThis.__neo_pending_action = { type: 'navigate', url: target.href };
        }
        return true;
    }
});
// Can't easily replace location on globalThis, but SPAs typically use history.pushState
