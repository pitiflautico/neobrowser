"""Adapters for each tool under test."""

import json, subprocess, time, sys, os, importlib.util
from pathlib import Path

# ── NeoBrowser adapter ──

class NeoBrowserAdapter:
    """Drives neobrowser via direct Python import."""

    def __init__(self):
        self.name = 'neobrowser'
        self._mod = None

    def start(self):
        """Import and initialize neobrowser module."""
        import types
        from unittest.mock import MagicMock
        # Stub websockets if needed (module handles its own launch)
        neo_path = Path(__file__).parent.parent / 'tools' / 'v3' / 'neo-browser.py'
        if str(neo_path.parent) not in sys.path:
            sys.path.insert(0, str(neo_path.parent))
        spec = importlib.util.spec_from_file_location('neo_browser_bench', str(neo_path))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        self._mod = mod
        return self

    def stop(self):
        if self._mod:
            try: self._mod.cleanup()
            except: pass

    def dispatch(self, tool, args):
        return self._mod.dispatch_tool(tool, args)

    def browse(self, url):
        return self.dispatch('browse', {'url': url})

    def open(self, url):
        return self.dispatch('open', {'url': url})

    def read(self, **kwargs):
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

    def __init__(self):
        self.name = 'playwright'
        self._pw = None
        self._browser = None
        self._page = None

    def start(self):
        from playwright.sync_api import sync_playwright
        self._pw = sync_playwright().start()
        self._browser = self._pw.chromium.launch(headless=True)
        self._page = self._browser.new_page()
        return self

    def stop(self):
        try:
            if self._browser: self._browser.close()
            if self._pw: self._pw.stop()
        except: pass

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
            self._page.click(f'text="{text}"', timeout=5000)
            return 'clicked'
        except:
            return 'click failed'

    def fill(self, selector='', value=''):
        try:
            self._page.fill(selector, value, timeout=5000)
            return 'filled'
        except:
            return 'fill failed'

    def submit(self):
        try:
            self._page.press('input', 'Enter')
            return 'submitted'
        except:
            return 'submit failed'

    def extract(self, type='links'):
        if type == 'links':
            return self._page.evaluate('() => Array.from(document.querySelectorAll("a[href]")).slice(0,30).map(a => a.href).join("\\n")')
        return self._page.inner_text('body')[:3000]

    def js(self, code):
        return str(self._page.evaluate(code))

    def screenshot(self):
        data = self._page.screenshot()
        return f'screenshot: {len(data)} bytes'


class FetchAdapter:
    """Plain HTTP fetch for baseline."""

    def __init__(self):
        self.name = 'fetch'

    def start(self):
        return self

    def stop(self):
        pass

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
