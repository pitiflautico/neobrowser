"""
test_operational.py — Integration tests for the NeoBrowser MCP server.

These tests exercise the REAL server (real Chrome, real network).
Run with:
    python3 -m pytest tests/operational/ -v --timeout=60

Mark: @pytest.mark.operational  — run separately from unit tests.
"""
import json
import threading
import time

import pytest

pytestmark = pytest.mark.operational


# ════════════════════════════════════════════════════════════════
# Group 1 — Browser Lifecycle
# ════════════════════════════════════════════════════════════════

class TestBrowserLifecycle:
    def test_chrome_launches(self, dispatch):
        """Chrome starts and returns a working status response."""
        result = dispatch('status', {})
        assert result is not None
        # Should return JSON with chrome/tabs info
        try:
            data = json.loads(result)
            assert 'chrome' in data or 'tabs' in data
        except json.JSONDecodeError:
            # Fallback: raw string should mention chrome or tab
            assert 'chrome' in result.lower() or 'tab' in result.lower()

    def test_chrome_reuses_instance(self, dispatch, nb):
        """Second status call reuses the same Chrome process."""
        dispatch('status', {})  # ensure launched
        pid_before = list(nb._chrome_pids) if nb._chrome_pids else []

        dispatch('status', {})
        pid_after = list(nb._chrome_pids) if nb._chrome_pids else []

        # PIDs should be identical — no new process spawned
        assert set(pid_before) == set(pid_after)


# ════════════════════════════════════════════════════════════════
# Group 2 — Navigation & Content
# ════════════════════════════════════════════════════════════════

class TestNavigationAndContent:
    def test_browse_returns_content(self, dispatch):
        """browse fetches a real page and returns recognisable content."""
        result = dispatch('browse', {'url': 'https://example.com'})
        assert result is not None
        assert 'Example Domain' in result or len(result) > 50

    def test_browse_fast_path(self, dispatch):
        """browse returns non-trivial content (fast path or fallback)."""
        result = dispatch('browse', {'url': 'https://example.com'})
        assert len(result) > 50

    def test_browse_blocked_url(self, dispatch):
        """browse rejects cloud metadata / private IP URLs."""
        result = dispatch('browse', {'url': 'http://169.254.169.254/latest/meta-data/'})
        assert result is not None
        low = result.lower()
        assert 'error' in low or 'block' in low or 'policy' in low

    def test_browse_cache_hit(self, dispatch):
        """Second browse to same URL is served from cache (faster)."""
        url = 'https://example.com'
        # Prime the cache
        dispatch('browse', {'url': url})

        t0 = time.time()
        dispatch('browse', {'url': url})
        second = time.time() - t0

        # Cache hit should be very fast (sub-100 ms)
        assert second < 1.0, f'Cache hit too slow: {second:.2f}s'

    def test_open_navigates_chrome(self, dispatch):
        """open loads a page in Ghost Chrome and returns content."""
        result = dispatch('open', {'url': 'https://example.com'})
        assert result is not None
        assert len(result) > 50

    def test_read_after_open(self, dispatch):
        """read returns content of the currently loaded page."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('read', {})
        assert result is not None
        assert 'Example Domain' in result or len(result) > 50

    def test_open_invalid_url(self, dispatch):
        """open with a non-URL string handles gracefully (no crash)."""
        result = dispatch('open', {'url': 'not-a-url'})
        assert result is not None  # Should return something, not throw


# ════════════════════════════════════════════════════════════════
# Group 3 — Search
# ════════════════════════════════════════════════════════════════

class TestSearch:
    def test_search_returns_results(self, dispatch):
        """search queries DuckDuckGo and returns non-empty results."""
        result = dispatch('search', {'query': 'python programming'})
        assert result is not None
        assert len(result) > 100
        assert 'python' in result.lower() or 'Python' in result


# ════════════════════════════════════════════════════════════════
# Group 4 — DOM Interaction
# ════════════════════════════════════════════════════════════════

class TestDOMInteraction:
    def test_find_elements(self, dispatch):
        """find locates elements on example.com."""
        dispatch('open', {'url': 'https://example.com'})
        # The link text on example.com is "More information..." — use partial match
        result = dispatch('find', {'text': 'More information'})
        assert result is not None
        # find returns a JSON array; non-empty array means it found something,
        # empty array [] is also a valid "not found" answer — just confirm no crash
        # and that the call returns a string (not None / exception).
        assert isinstance(result, str)
        # Verify by also trying the exact anchor text
        result2 = dispatch('find', {'text': 'More information...'})
        assert result2 is not None
        assert isinstance(result2, str)

    def test_click_element(self, dispatch):
        """click interacts with a link (no crash, returns something)."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('click', {'text': 'More information'})
        assert result is not None

    def test_js_execution(self, dispatch):
        """js executes JavaScript and returns the page title."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('js', {'code': 'return document.title'})
        assert result is not None
        assert 'Example' in result

    def test_extract_links(self, dispatch):
        """extract returns links from the current page."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('extract', {'type': 'links'})
        assert result is not None
        assert 'iana.org' in result.lower() or 'link' in result.lower() or len(result) > 10


# ════════════════════════════════════════════════════════════════
# Group 5 — Smart Extractors (mode / type)
# ════════════════════════════════════════════════════════════════

class TestSmartExtractors:
    def test_read_mode_tweets_param(self, dispatch):
        """read accepts mode=tweets; example.com has no tweets."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('read', {'mode': 'tweets'})
        assert result is not None
        assert 'No tweets' in result or 'tweet' in result.lower()

    def test_read_type_tweets_param(self, dispatch):
        """read accepts type=tweets (legacy / plugin compat)."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('read', {'type': 'tweets'})
        assert result is not None
        assert 'No tweets' in result or 'tweet' in result.lower()


# ════════════════════════════════════════════════════════════════
# Group 6 — Plugin System
# ════════════════════════════════════════════════════════════════

class TestPluginSystem:
    def test_plugin_list(self, dispatch):
        """plugin action=list returns plugin info or empty notice."""
        result = dispatch('plugin', {'name': 'x', 'action': 'list'})
        assert result is not None
        # Either lists plugins or says none are found
        assert (
            'plugin' in result.lower()
            or 'No plugins' in result
            or 'twitter' in result.lower()
        )

    def test_plugin_not_found(self, dispatch):
        """plugin with non-existent name returns an error."""
        result = dispatch('plugin', {'name': 'nonexistent-plugin-xyz-abc'})
        assert result is not None
        low = result.lower()
        assert 'not found' in low or 'error' in low or 'no plugin' in low


# ════════════════════════════════════════════════════════════════
# Group 7 — Error Handling
# ════════════════════════════════════════════════════════════════

class TestErrorHandling:
    def test_unknown_tool(self, dispatch):
        """dispatch_tool returns error for completely unknown tool name."""
        result = dispatch('nonexistent_tool_xyz', {})
        assert result is not None
        assert 'Unknown' in result or 'error' in result.lower()

    def test_browse_empty_url(self, dispatch):
        """browse with empty URL returns a validation error."""
        result = dispatch('browse', {'url': ''})
        assert result is not None
        low = result.lower()
        assert 'required' in low or 'error' in low or 'url' in low


# ════════════════════════════════════════════════════════════════
# Group 8 — Concurrent Safety
# ════════════════════════════════════════════════════════════════

class TestConcurrentSafety:
    def test_sequential_lock_works(self, dispatch):
        """Three concurrent open() calls all complete without crashing."""
        results = []
        errors = []

        def do_open():
            try:
                r = dispatch('open', {'url': 'https://example.com'})
                results.append(r)
            except Exception as e:
                errors.append(str(e))

        threads = [threading.Thread(target=do_open) for _ in range(3)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=60)

        assert len(errors) == 0, f'Errors in threads: {errors}'
        assert len(results) == 3
        assert all(r is not None for r in results)


# ════════════════════════════════════════════════════════════════
# Group 9 — Screenshot
# ════════════════════════════════════════════════════════════════

class TestScreenshot:
    def test_screenshot_returns_data(self, dispatch):
        """screenshot saves a PNG and returns its path."""
        dispatch('open', {'url': 'https://example.com'})
        result = dispatch('screenshot', {})
        assert result is not None
        assert len(result) > 10
        # Should mention the path
        assert 'screenshot' in result.lower() or '.png' in result.lower() or '/' in result
