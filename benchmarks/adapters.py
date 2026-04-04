"""Adapters for each tool under test."""

import json, subprocess, time, sys, os, importlib.util
from pathlib import Path

# ── NeoBrowser adapter ──

class NeoBrowserAdapter:
    """Drives neobrowser via direct Python import.

    Import is deferred to start() so that merely constructing the adapter
    does not launch Chrome. If the module or Chrome is unavailable, start()
    raises ImportError and the caller should skip this tool.
    """

    name = 'neobrowser'

    def __init__(self):
        self._mod = None

    def start(self):
        """Import and initialize neobrowser module. Raises if unavailable."""
        neo_path = Path(__file__).parent.parent / 'tools' / 'v3' / 'neo-browser.py'
        if not neo_path.exists():
            raise ImportError(f'neo-browser.py not found at {neo_path}')
        if str(neo_path.parent) not in sys.path:
            sys.path.insert(0, str(neo_path.parent))
        try:
            spec = importlib.util.spec_from_file_location('neo_browser_bench', str(neo_path))
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
        except Exception as exc:
            raise ImportError(f'Failed to load neo-browser: {exc}') from exc
        self._mod = mod
        return self

    def prewarm(self):
        """Start Chrome in background — call this after cold_start tests, before spa_heavy.
        Does NOT block: Chrome warms up while warm_run and static_page tests run.
        """
        if self._mod and hasattr(self._mod, 'prewarm_chrome'):
            try:
                self._mod.prewarm_chrome()
            except Exception:
                pass

    def stop(self):
        if self._mod:
            try:
                self._mod.cleanup()
            except Exception:
                pass

    def clear_cache(self):
        """Reset the in-process page cache so next fetch goes to the network."""
        if self._mod and hasattr(self._mod, '_page_cache'):
            cache = self._mod._page_cache
            if hasattr(cache, 'clear'):
                cache.clear()
            elif hasattr(cache, '_cache'):
                # Fallback for older versions without clear() method
                import threading
                with getattr(cache, '_lock', threading.Lock()):
                    cache._cache.clear()

    def dispatch(self, tool, args):
        return self._mod.dispatch_tool(tool, args)

    def browse(self, url):
        return self.dispatch('browse', {'url': url})

    def open(self, url):
        return self.dispatch('open', {'url': url})

    def read(self, **kwargs):
        # Default to 'text' type (fast JS innerText) so spa_heavy benchmark
        # measures page extraction speed fairly vs playwright's inner_text()
        if 'type' not in kwargs and 'mode' not in kwargs:
            kwargs['type'] = 'text'
        return self.dispatch('read', kwargs)

    def find(self, text):
        return self.dispatch('find', {'text': text})

    def click(self, text):
        return self.dispatch('click', {'text': text})

    def fill(self, selector='', value=''):
        return self.dispatch('fill', {'selector': selector, 'value': value})

    def submit(self):
        return self.dispatch('submit', {})

    def extract(self, type='links'):
        return self.dispatch('extract', {'type': type})

    def js(self, code):
        return self.dispatch('js', {'code': code})

    def screenshot(self):
        return self.dispatch('screenshot', {})


class PlaywrightAdapter:
    """Drives Playwright for comparison."""

    name = 'playwright'

    def __init__(self):
        self._pw = None
        self._browser = None
        self._page = None

    def start(self):
        try:
            from playwright.sync_api import sync_playwright
        except ImportError as exc:
            raise ImportError('playwright not installed: pip install playwright && playwright install chromium') from exc
        self._pw = sync_playwright().start()
        self._browser = self._pw.chromium.launch(headless=True)
        self._page = self._browser.new_page()
        return self

    def stop(self):
        try:
            if self._browser:
                self._browser.close()
            if self._pw:
                self._pw.stop()
        except Exception:
            pass

    def clear_cache(self):
        """Playwright has its own network layer; clear page state."""
        try:
            if self._page:
                self._page.evaluate('() => { window.sessionStorage.clear(); window.localStorage.clear(); }')
        except Exception:
            pass

    def browse(self, url):
        self._page.goto(url, wait_until='domcontentloaded', timeout=30000)
        return self._page.content()[:5000]

    def open(self, url):
        self._page.goto(url, wait_until='domcontentloaded', timeout=30000)
        return self._page.title()

    def read(self, **kwargs):
        return self._page.inner_text('body')[:5000]

    def find(self, text):
        els = self._page.query_selector_all(f'text="{text}"')
        return f'Found {len(els)} elements'

    def click(self, text):
        try:
            # Use partial text match (no quotes) so "More information" matches
            # "More information..." — quoted form requires exact match in playwright
            self._page.click(f'text={text}', timeout=2000)
            return 'clicked'
        except Exception:
            return 'click failed'

    def fill(self, selector='', value=''):
        try:
            self._page.fill(selector, value, timeout=5000)
            return 'filled'
        except Exception:
            return 'fill failed'

    def submit(self):
        try:
            self._page.press('input', 'Enter')
            return 'submitted'
        except Exception:
            return 'submit failed'

    def extract(self, type='links'):
        if type == 'links':
            return self._page.evaluate(
                '() => Array.from(document.querySelectorAll("a[href]")).slice(0,30).map(a => a.href).join("\\n")'
            )
        return self._page.inner_text('body')[:3000]

    def js(self, code):
        return str(self._page.evaluate(code))

    def screenshot(self):
        data = self._page.screenshot()
        return f'screenshot: {len(data)} bytes'


class NeoBrowserOrigAdapter(NeoBrowserAdapter):
    """Drives the last-committed (unmodified) neo-browser V3 for comparison.

    Uses git show HEAD:tools/v3/neo-browser.py saved to /tmp/neo_orig.py.
    Run: git show HEAD:tools/v3/neo-browser.py > /tmp/neo_orig.py first.
    """

    name = 'neo-orig'

    def start(self):
        orig_path = Path('/tmp/neo_orig.py')
        if not orig_path.exists():
            raise ImportError('neo_orig.py not found at /tmp/neo_orig.py — run: git show HEAD:tools/v3/neo-browser.py > /tmp/neo_orig.py')
        try:
            spec = importlib.util.spec_from_file_location('neo_browser_orig', str(orig_path))
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
        except Exception as exc:
            raise ImportError(f'Failed to load neo-orig: {exc}') from exc
        self._mod = mod
        return self


class FetchAdapter:
    """Plain HTTP fetch for baseline."""

    name = 'fetch'

    def start(self):
        return self

    def stop(self):
        pass

    def clear_cache(self):
        pass  # stateless — nothing to clear

    def browse(self, url):
        import urllib.request, re
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        body = urllib.request.urlopen(req, timeout=10).read().decode('utf-8', errors='replace')
        text = re.sub(r'<[^>]+>', ' ', body)
        return re.sub(r'\s+', ' ', text).strip()[:5000]

    def open(self, url):
        return self.browse(url)

    def read(self, **kwargs):
        return 'N/A for fetch'

    def find(self, text):
        return 'N/A for fetch'

    def click(self, text):
        return 'N/A for fetch'

    def fill(self, selector='', value=''):
        return 'N/A for fetch'

    def submit(self):
        return 'N/A for fetch'

    def extract(self, type='links'):
        return 'N/A for fetch'

    def js(self, code):
        return 'N/A for fetch'

    def screenshot(self):
        return 'N/A for fetch'
