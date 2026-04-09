"""
tools/v4/server.py

NeoBrowser V4 — MCP Server (stdin/stdout JSON-RPC 2.0)

Exposes the Browser facade as MCP tools. Separate entry point from V3
so both can run simultaneously for A/B comparison.

Tools exposed:
  navigate      — open URL in Chrome (tab pool reuse)
  screenshot    — capture page as PNG base64
  read          — extract page text via JS
  find          — find element by intent (AX tree + LLM)
  click         — click element by backendNodeId or selector
  type          — type text into focused element (CDP Input.insertText)
  console_logs  — get captured console log entries
  network_log   — get captured network requests
  metrics       — get Chrome performance metrics
  save_cookies  — persist session cookies to disk
  restore_cookies — inject saved cookies into tab
  record_task   — start recording a playbook
  stop_recording — stop recording, save playbook
  replay        — replay a saved playbook

Usage:
  python3 tools/v4/server.py             # start MCP server
  python3 tools/v4/server.py --version   # print version
  python3 tools/v4/server.py doctor      # check deps

Claude Code config (alongside V3):
  {
    "neo-browser-v3": {"command": "python3", "args": ["tools/v3/neo-browser.py"]},
    "neo-browser-v4": {"command": "python3", "args": ["tools/v4/server.py"]}
  }
"""
from __future__ import annotations

import json
import sys
import os
import traceback
import logging
from typing import Any

# Ensure project root on path when run directly
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", ".."))

VERSION = "4.0.0"
SERVER_NAME = "neo-browser-v4"
log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Global Browser instance — one per server process
# ---------------------------------------------------------------------------

_browser = None  # type: ignore[assignment]
_current_tab = None  # type: ignore[assignment]
_chat_pipelines: dict = {}  # platform → ChatPipeline instance


def _resolve_attach_port() -> int | None:
    """
    Resolve which Chrome port to attach to, in priority order:
    1. NEOBROWSER_ATTACH_PORT env var (explicit override)
    2. ~/.neorender/neo-browser-port.txt written by V3 (dynamic, read at call time)
    Returns None if no reachable Chrome is found.
    """
    import urllib.request as _ur

    def _reachable(port: int) -> bool:
        try:
            _ur.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.0)
            return True
        except Exception:
            return False

    # 1. Explicit env var
    env_port = os.environ.get("NEOBROWSER_ATTACH_PORT")
    if env_port:
        p = int(env_port)
        if _reachable(p):
            return p

    # 2. V3 port file (re-read every time — V3 Chrome may have restarted)
    port_file = os.path.expanduser("~/.neorender/neo-browser-port.txt")
    if os.path.exists(port_file):
        try:
            p = int(open(port_file).read().strip())
            if _reachable(p):
                return p
        except Exception:
            pass

    return None


def _get_browser():
    global _browser
    if _browser is None:
        from tools.v4.browser import Browser
        pool_size = int(os.environ.get("NEOBROWSER_POOL_SIZE", "3"))
        attach_port = _resolve_attach_port()
        if attach_port:
            _browser = Browser.connect(attach_port, pool_size=pool_size)
            log.info("Browser attached (port=%s, pool=%d)", attach_port, pool_size)
        else:
            profile = os.environ.get("NEOBROWSER_PROFILE", "default")
            _browser = Browser(profile=profile, pool_size=pool_size)
            log.info("Browser started (profile=%s, pool=%d)", profile, pool_size)
    return _browser


def _get_tab(url: str | None = None, wait_s: float = 3.0):
    """Get current tab, navigating if url provided. Auto-recovers stale WebSocket or dead Chrome."""
    global _current_tab, _browser

    def _fresh_browser():
        """Re-resolve Chrome port and create new Browser instance."""
        global _browser
        _browser = None
        return _get_browser()

    b = _get_browser()
    if _current_tab is None:
        _current_tab = b.open(url or "about:blank", wait_s=wait_s if url else 0)
    elif url:
        try:
            _current_tab.navigate(url, wait_s=wait_s)
        except Exception:
            # WebSocket died or Chrome restarted — re-resolve and open fresh tab
            _current_tab = None
            b = _fresh_browser()
            _current_tab = b.open(url, wait_s=wait_s)
    else:
        # No navigation — ping to detect stale WebSocket
        try:
            _current_tab.js("return 1")
        except Exception:
            # WebSocket/Chrome died — re-resolve Chrome, reopen on saved URL
            saved_url = "about:blank"
            try:
                saved_url = _current_tab.current_url() or "about:blank"
            except Exception:
                pass
            _current_tab = None
            b = _fresh_browser()
            try:
                _current_tab = b.open(saved_url, wait_s=2)
            except Exception:
                _current_tab = b.open("about:blank", wait_s=0)
    return _current_tab


# ---------------------------------------------------------------------------
# Tool definitions
# ---------------------------------------------------------------------------

TOOLS = {
    "navigate": {
        "description": "Open URL in Chrome (V4: tab pool reuse, AX cache, thread-safe). Required for SPAs, JS-heavy sites, and login-required pages.",
        "schema": {
            "url":    {"type": "string",  "description": "HTTP/HTTPS URL to open", "required": True},
            "wait_s": {"type": "number",  "description": "Seconds to wait for page render (default 3.0)"},
        },
    },
    "screenshot": {
        "description": "Capture current page viewport as base64 PNG. V4: also supports JPEG.",
        "schema": {
            "format":  {"type": "string", "description": "Image format: png (default) or jpeg"},
            "quality": {"type": "integer","description": "JPEG quality 0-100 (default 80, ignored for PNG)"},
        },
    },
    "read": {
        "description": "Extract visible text from current page via JavaScript.",
        "schema": {
            "selector": {"type": "string", "description": "Optional CSS selector to read specific element (default: body)"},
        },
    },
    "find": {
        "description": "Find UI element by natural language intent. Uses AX tree + heuristics + LLM. Returns backendNodeId for use with click.",
        "schema": {
            "intent": {"type": "string", "description": "What to find, e.g. 'send button', 'message input box'", "required": True},
        },
    },
    "click": {
        "description": "Click element by backendNodeId (from find) or CSS selector.",
        "schema": {
            "backend_node_id": {"type": "integer", "description": "backendNodeId from find result"},
            "selector":        {"type": "string",  "description": "CSS selector fallback"},
        },
    },
    "type": {
        "description": "Type text into currently focused element. Uses CDP Input.insertText — works with React/Vue SPAs.",
        "schema": {
            "text": {"type": "string", "description": "Text to type", "required": True},
        },
    },
    "console_logs": {
        "description": "Get captured browser console log entries (log/warning/error/exception). V4 only.",
        "schema": {
            "level": {"type": "string", "description": "Filter by level: log, info, warning, error (default: all)"},
            "limit": {"type": "integer","description": "Max entries to return (default 50)"},
        },
    },
    "network_log": {
        "description": "Get captured network requests with status, duration, size. V4 only.",
        "schema": {
            "url_pattern": {"type": "string", "description": "Filter by URL substring (default: all)"},
            "limit":       {"type": "integer","description": "Max entries (default 50)"},
        },
    },
    "metrics": {
        "description": "Get Chrome performance metrics: JSHeapUsedSize, Nodes, Documents, etc. V4 only.",
        "schema": {
            "key": {"type": "string", "description": "Return only this metric (default: all)"},
        },
    },
    "save_cookies": {
        "description": "Save current session cookies to ~/.neorender/cookies/{profile}.json (0600 perms). V4 only.",
        "schema": {},
    },
    "restore_cookies": {
        "description": "Inject saved cookies from disk into current tab. Returns count restored. V4 only.",
        "schema": {},
    },
    "save_session": {
        "description": "Full session save: cookies + localStorage → ~/.neorender/sessions/. Persists authenticated state so future V4 restarts are pre-authenticated. V4 only.",
        "schema": {},
    },
    "session_info": {
        "description": "Show session persistence state: last sync time, cookie count, domains, file paths. V4 only.",
        "schema": {},
    },
    "record_task": {
        "description": "Start recording interaction steps as a playbook for future replay. V4 only.",
        "schema": {
            "domain":    {"type": "string", "description": "Domain key, e.g. 'linkedin.com'", "required": True},
            "task_name": {"type": "string", "description": "Task identifier, e.g. 'send_message'", "required": True},
        },
    },
    "stop_recording": {
        "description": "Stop recording and save playbook to disk. Returns step count. V4 only.",
        "schema": {},
    },
    "replay": {
        "description": "Replay a saved playbook. Returns {ok, first_failed_step}. V4 only.",
        "schema": {
            "domain":    {"type": "string", "description": "Domain key", "required": True},
            "task_name": {"type": "string", "description": "Task name", "required": True},
        },
    },
    "scroll": {
        "description": "Scroll the current page. Use to reach content below the fold or trigger lazy loading.",
        "schema": {
            "direction": {"type": "string", "description": "Scroll direction: down (default), up, top, bottom", "enum": ["down", "up", "top", "bottom"]},
            "amount":    {"type": "integer", "description": "Pixels to scroll (default 500, ignored for top/bottom)"},
        },
    },
    "wait": {
        "description": "Wait for a condition or fixed duration. Use to let content load or streaming finish.",
        "schema": {
            "ms":       {"type": "integer", "description": "Milliseconds to wait (default 1000)"},
            "selector": {"type": "string",  "description": "Optional CSS selector — wait until it appears (up to ms timeout)"},
        },
    },
    "js": {
        "description": "Execute JavaScript in the current page and return the result. Code must use 'return' to return a value.",
        "schema": {
            "code": {"type": "string", "description": "JavaScript code to execute. Must use return statement.", "required": True},
        },
    },
    "page_info": {
        "description": "Quick orientation: current URL, title, page state, interactive element count, form count, overlay detection. Returns <200 tokens in <200ms.",
        "schema": {},
    },
    "status": {
        "description": "Browser status: current tab URL, title, open tab count, Ghost Chrome PID.",
        "schema": {},
    },
    "analyze": {
        "description": "Semantic page map: forms (fields, labels, actions), buttons, overlays, active input. Use before fill/form_fill to understand page structure.",
        "schema": {},
    },
    "fill": {
        "description": "Smart fill for a single form field. Supports input, textarea, select, checkbox, radio. React/Vue compatible (fires synthetic events).",
        "schema": {
            "selector": {"type": "string", "description": "CSS selector for the field", "required": True},
            "value":    {"type": "string", "description": "Value to fill", "required": True},
        },
    },
    "form_fill": {
        "description": "Fill multiple form fields in one call using fuzzy label matching. Pass a dict of {label: value} pairs.",
        "schema": {
            "fields":     {"type": "object", "description": "Dict of {label_or_placeholder: value} pairs", "required": True},
            "form_index": {"type": "integer","description": "Which form to target if multiple on page (default: 0)"},
        },
    },
    "submit": {
        "description": "Submit the current form. Clicks submit button or calls form.submit().",
        "schema": {
            "selector": {"type": "string", "description": "CSS selector for submit button (auto-detected if omitted)"},
        },
    },
    "find_and_click": {
        "description": "Find element by text/label using AX tree and click it. More reliable than click+selector for dynamic UIs.",
        "schema": {
            "text":     {"type": "string", "description": "Visible text or label to search for", "required": True},
            "role":     {"type": "string", "description": "Optional ARIA role filter: button, link, menuitem, etc."},
            "nth":      {"type": "integer","description": "Which match to click if multiple (0-indexed, default 0)"},
        },
    },
    "login": {
        "description": "Automated login: navigate to URL, fill email+password, submit. Returns session state.",
        "schema": {
            "url":      {"type": "string", "description": "Login page URL", "required": True},
            "email":    {"type": "string", "description": "Email or username", "required": True},
            "password": {"type": "string", "description": "Password", "required": True},
        },
    },
    "extract": {
        "description": "Extract structured data from page: links or tables as text.",
        "schema": {
            "what": {"type": "string", "description": "What to extract: links, tables (default: links)", "enum": ["links", "tables"]},
        },
    },
    "extract_table": {
        "description": "Extract HTML table as JSON array of objects. Keys are column headers.",
        "schema": {
            "selector": {"type": "string", "description": "CSS selector for table element (default: first table)"},
            "index":    {"type": "integer","description": "Table index if selector matches multiple (default: 0)"},
        },
    },
    "paginate": {
        "description": "Navigate to next page using common next-page patterns (Next button, arrow, page number).",
        "schema": {
            "selector": {"type": "string", "description": "CSS selector for next button (auto-detected if omitted)"},
        },
    },
    "dismiss_overlay": {
        "description": "Detect and dismiss cookie banners, GDPR modals, popups. Clicks Accept/Close/Reject.",
        "schema": {},
    },
    "browse": {
        "description": "Fast HTTP fetch without Chrome (no JS). Use for static pages, APIs, sitemaps. Falls back to Chrome for JS-heavy pages.",
        "schema": {
            "url":     {"type": "string", "description": "URL to fetch", "required": True},
            "headers": {"type": "object", "description": "Optional request headers"},
        },
    },
    "search": {
        "description": "Web search via DuckDuckGo. Returns top results with title, URL, snippet.",
        "schema": {
            "query": {"type": "string", "description": "Search query", "required": True},
            "limit": {"type": "integer","description": "Max results (default 10)"},
        },
    },
    "debug": {
        "description": "Capture browser console errors/logs. Installs interceptor and flushes buffered messages.",
        "schema": {
            "action": {"type": "string", "description": "Action: start (install interceptor), flush (get buffered logs), stop", "enum": ["start", "flush", "stop"]},
        },
    },
    "gpt": {
        "description": "Chat with ChatGPT using the user's real Chrome session (no API key). Requires user to be logged into chatgpt.com in Chrome. Actions: send (default), read_last, is_streaming, history, check_session.",
        "schema": {
            "message": {"type": "string",  "description": "Message to send to ChatGPT"},
            "action":  {"type": "string",  "description": "Action: send (default), read_last, is_streaming, history, check_session, debug_network, debug_watch", "enum": ["send", "read_last", "is_streaming", "history", "check_session", "debug_network", "debug_watch"]},
            "wait":    {"type": "boolean", "description": "Wait for full response before returning (default true)"},
        },
    },
    "grok": {
        "description": "Chat with Grok (X.com) using the user's real Chrome session (no API key). Same interface as gpt. Requires login to X.com in Chrome.",
        "schema": {
            "message": {"type": "string",  "description": "Message to send to Grok"},
            "action":  {"type": "string",  "description": "Action: send (default), read_last, is_streaming, history", "enum": ["send", "read_last", "is_streaming", "history"]},
        },
    },
}


# ---------------------------------------------------------------------------
# Chat pipeline — full port of v3 ChatPipeline to v4 primitives
# ---------------------------------------------------------------------------

class ChatPipeline:
    """
    Closed pipeline for chat platforms (ChatGPT, Grok).
    Mirrors v3 ChatPipeline but uses v4 tab.js() / tab.send() primitives.

    Key design:
    - Input/send button are discovered dynamically via FormFinder (LLM+heuristics),
      not hardcoded CSS selectors that break on every UI update.
    - Auth state is checked via local LLM (Gemma 4 at localhost:8080), not text regex.
    - Discovered selectors are cached per session to avoid re-discovery overhead.

    Steps: ensure → verify_ready → send → wait_response
    """

    # Platform metadata — only stable semantic selectors here (data-attributes).
    # Input/send button are discovered dynamically. These are fallbacks only.
    PLATFORMS: dict = {
        "gpt": {
            "url": "https://chatgpt.com",
            "input_hint": "message input textarea",
            "send_hint": "send message button",
            "input_fallback": "#prompt-textarea",
            "send_fallback": "[data-testid=send-button]",
            "assistant": "[data-message-author-role=assistant]",
            "user": "[data-message-author-role=user]",
            "stop_btn": "[data-testid=stop-button]",
        },
        "grok": {
            "url": "https://x.com/i/grok",
            "input_hint": "message input textarea",
            "send_hint": "send message button",
            "input_fallback": "textarea",
            "send_fallback": "button[type=submit]",
            "assistant": "div.prose, .markdown",
            "user": None,
            "stop_btn": None,
        },
    }

    # Local LLM endpoint (llama.cpp / Gemma 4 E2B)
    _LLM_URL = "http://localhost:8080/v1/chat/completions"

    def __init__(self, platform: str) -> None:
        import threading
        self.platform = platform
        self.conv_url: str | None = None
        self._tab = None
        self._last_error: str = ""
        self._msg_count_before: int = 0
        self._last_text_before: str = ""
        self._send_verified: bool = False
        self._lock = threading.Lock()
        # Cached discovered selectors (reset when tab navigates to new URL)
        self._sel_cache: dict = {}  # {"input": "css...", "send_btn": "css..."}
        self._sel_cache_url: str = ""
        # Network watcher queue registered in send(), consumed in wait_response()
        self._net_watcher_q = None

    @property
    def _cfg(self) -> dict:
        return self.PLATFORMS[self.platform]

    @property
    def _log(self):
        return logging.getLogger(f"neo.chat.{self.platform}")

    def _js(self, code: str):
        return self._tab.js(code)

    def _cdp(self, method: str, params: dict):
        return self._tab.send(method, params)

    # ── LLM helpers ───────────────────────────────────────────────────────────

    def _llm_ask(self, prompt: str, max_tokens: int = 20) -> str:
        """Ask local Gemma 4 a question. Returns text or '' on failure."""
        import urllib.request as _ur
        try:
            body = json.dumps({
                "model": "local",
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": max_tokens,
                "temperature": 0,
            }).encode()
            req = _ur.Request(self._LLM_URL, data=body,
                              headers={"Content-Type": "application/json"})
            resp = json.loads(_ur.urlopen(req, timeout=8).read())
            return resp["choices"][0]["message"]["content"].strip()
        except Exception as e:
            self._log.debug("_llm_ask failed: %s", e)
            return ""

    def _is_logged_in(self) -> bool:
        """
        Ask local LLM whether the current page shows an authenticated session.
        Falls back to DOM check (login button present) if LLM is unavailable.
        """
        log = self._log
        page_text = self._js("return document.body?.innerText?.substring(0,800)") or ""

        answer = self._llm_ask(
            f"Page text from a browser tab:\n---\n{page_text[:600]}\n---\n"
            f"Is the user currently logged into a chat service (not seeing a login/signup page)? "
            f"Answer only YES or NO.",
            max_tokens=5,
        )
        if answer:
            logged_in = answer.upper().startswith("Y")
            log.info("[auth] LLM says logged_in=%s (answer=%r)", logged_in, answer)
            return logged_in

        # LLM unavailable — fall back to DOM: login button absent = logged in
        has_login = bool(self._js(
            "return !!(document.querySelector('[data-testid=\"login-button\"]') || "
            "document.querySelector('a[href*=\"/auth/login\"]'))"
        ))
        log.info("[auth] LLM unavailable, DOM fallback: has_login_btn=%s", has_login)
        return not has_login

    # ── Dynamic selector discovery ────────────────────────────────────────────

    def _discover_selectors(self) -> dict:
        """
        Use FormFinder (LLM + heuristics + AX tree) to find input and send button.
        Results are cached per URL so discovery only runs once per page load.
        Returns {"input": str, "send_btn": str}.
        """
        log = self._log
        cfg = self._cfg
        current_url = (self._js("return location.href") or "").split("?")[0]

        if self._sel_cache and self._sel_cache_url == current_url:
            log.debug("[discover] cache hit for %s", current_url[:60])
            return self._sel_cache

        log.info("[discover] running FormFinder for input + send_btn")
        try:
            from tools.v4.page_analyzer import FormFinder
            finder = FormFinder(self._tab)

            result_input = finder.find(cfg["input_hint"])
            result_send = finder.find(cfg["send_hint"])

            sel_input = (result_input.selector if result_input else None) or cfg["input_fallback"]
            sel_send = (result_send.selector if result_send else None) or cfg["send_fallback"]

            # Validate input: must be contenteditable (ProseMirror) or a textarea,
            # NOT a search/filter input. ChatGPT's search box (#search-ui-input-*)
            # can be mistakenly picked by FormFinder.
            if sel_input and sel_input != cfg["input_fallback"]:
                is_valid = self._js(f"""
                    const el = document.querySelector({json.dumps(sel_input)});
                    if (!el) return false;
                    const ce = el.getAttribute('contenteditable');
                    const tag = el.tagName;
                    const typ = (el.getAttribute('type') || '').toLowerCase();
                    // Reject search inputs; accept contenteditable or submit-adjacent textarea
                    return ce !== null || (tag === 'TEXTAREA' && typ !== 'search');
                """)
                if not is_valid:
                    log.warning("[discover] input %s rejected (not contenteditable), using fallback %s",
                                sel_input, cfg["input_fallback"])
                    sel_input = cfg["input_fallback"]

            log.info("[discover] input=%s (confidence=%s) send=%s (confidence=%s)",
                     sel_input, getattr(result_input, "confidence", "?"),
                     sel_send, getattr(result_send, "confidence", "?"))
        except Exception as e:
            log.warning("[discover] FormFinder failed (%s), using fallbacks", e)
            sel_input = cfg["input_fallback"]
            sel_send = cfg["send_fallback"]

        self._sel_cache = {"input": sel_input, "send_btn": sel_send}
        self._sel_cache_url = current_url
        return self._sel_cache

    def _invalidate_sel_cache(self) -> None:
        self._sel_cache = {}
        self._sel_cache_url = ""

    # ── Tab discovery ─────────────────────────────────────────────────────────

    def _find_existing_tab(self, domain: str):
        """
        Scan Chrome's open tabs via /json/list and attach to the first one
        that already has the platform's domain loaded.

        Returns a ChromeTab or None. This avoids cold-starting a new tab when
        the user already has chatgpt.com open from a previous session.
        """
        try:
            import urllib.request as _ur
            b = _get_browser()
            # Resolve Chrome port from the browser's session
            port = getattr(getattr(b, "_session", None), "_port", None)
            if port is None:
                return None
            resp = _ur.urlopen(f"http://localhost:{port}/json/list", timeout=3)
            tabs = json.loads(resp.read())
            for t in tabs:
                url = t.get("url", "")
                if domain in url and t.get("webSocketDebuggerUrl"):
                    from tools.v4.chrome_tab import ChromeTab
                    tab = ChromeTab.attach(t["webSocketDebuggerUrl"], t["id"], port)
                    self._log.info("reusing existing tab id=%s url=%s", t["id"][:8], url[:60])
                    return tab
        except Exception as e:
            self._log.debug("_find_existing_tab failed: %s", e)
        return None

    # ── Step 1: ensure tab, navigate, auth check ─────────────────────────────

    def ensure(self) -> bool:
        import time
        cfg = self._cfg
        log = self._log

        t0 = time.time()
        domain = cfg["url"].split("/")[2]

        # ── 1a. Get or reuse tab ──────────────────────────────────────────────
        if self._tab is None:
            existing = self._find_existing_tab(domain)
            if existing:
                self._tab = existing
                log.info("[ensure] reused existing tab (%.0fms)", (time.time()-t0)*1000)
            else:
                log.info("[ensure] opening new dedicated tab for %s", self.platform)
                self._tab = _get_browser().open("about:blank", wait_s=0)
                log.info("[ensure] tab opened (%.0fms)", (time.time()-t0)*1000)

        # ── 1b. Navigate if not on platform ───────────────────────────────────
        current_url = self._js("return location.href") or ""
        log.info("[ensure] current_url=%s", current_url[:80])

        if domain not in current_url:
            target = self.conv_url or cfg["url"]
            log.info("[ensure] navigating → %s", target)
            self._tab.navigate(target, wait_s=3.0)
            current_url = self._js("return location.href") or ""
            log.info("[ensure] post-nav url=%s (%.0fms)", current_url[:80], (time.time()-t0)*1000)
        elif self.conv_url and self.conv_url not in current_url:
            log.info("[ensure] tab drifted, restoring conv_url=%s", self.conv_url)
            self._tab.navigate(self.conv_url, wait_s=3.0)
            current_url = self._js("return location.href") or ""
        elif not self.conv_url and "/c/" in current_url:
            self.conv_url = current_url.split("?")[0]
            log.info("[ensure] adopted conversation url=%s", self.conv_url)

        # ── 1c. DOM readiness — wait for complete before proceeding ──────────────
        deadline_ready = time.time() + 10
        while time.time() < deadline_ready:
            ready_state = self._js("return document.readyState")
            if ready_state == "complete":
                break
            time.sleep(0.3)
        log.info("[ensure] readyState=%s (%.0fms)", ready_state, (time.time()-t0)*1000)

        # Invalidate selector cache on navigation (URL changed)
        if self._sel_cache_url and self._sel_cache_url not in current_url:
            log.info("[ensure] URL changed, invalidating selector cache")
            self._invalidate_sel_cache()

        # ── 1d. Error / Cloudflare / auth guards ──────────────────────────────
        if self._js("return !!(document.body?.innerText?.includes('Something went wrong'))"):
            log.warning("[ensure] error state, navigating fresh")
            self.conv_url = None
            self._invalidate_sel_cache()
            self._tab.navigate(cfg["url"], wait_s=4.0)

        cf = self._js("""return !!(document.querySelector('#challenge-form') ||
            document.querySelector('.cf-browser-verification') ||
            document.title === 'Just a moment...')""")
        if cf:
            log.warning("[ensure] Cloudflare challenge")
            self._last_error = json.dumps({"status": "error", "error": "cf_challenge",
                "suggestion": "Solve Cloudflare in real Chrome, then retry."})
            return False

        # ── 1e. Auth check via local LLM ──────────────────────────────────────
        logged_in = self._is_logged_in()
        log.info("[ensure] is_logged_in=%s (%.0fms)", logged_in, (time.time()-t0)*1000)
        if not logged_in:
            log.warning("[ensure] not authenticated")
            self._last_error = json.dumps({"status": "error", "error": "login_wall",
                "suggestion": "Log into the platform in your real Chrome browser, then retry."})
            return False

        log.info("[ensure] OK (%.0fms total)", (time.time()-t0)*1000)
        return True

    # ── Step 2: verify no streaming in progress, input available ─────────────

    def verify_ready(self) -> bool:
        import time
        cfg = self._cfg
        log = self._log
        t0 = time.time()
        stop_sel = cfg.get("stop_btn")

        log.info("[verify_ready] checking stop_btn=%s", stop_sel)
        if stop_sel and self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"):
            log.info("[verify_ready] streaming in progress, waiting up to 30s")
            for i in range(60):
                time.sleep(0.5)
                if not self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"):
                    log.info("[verify_ready] streaming stopped after %.0fs", i*0.5)
                    break
            else:
                log.warning("[verify_ready] still streaming after 30s")
                self._last_error = json.dumps({"status": "error", "error": "still_streaming",
                    "suggestion": "Use action=read_last to get current response, then retry."})
                return False

        # Discover input dynamically (FormFinder + LLM), fallback to hardcoded
        sels = self._discover_selectors()
        input_sel = sels["input"]
        has_input = self._js(f"return !!document.querySelector({json.dumps(input_sel)})")
        log.info("[verify_ready] input_sel=%s found=%s (%.0fms)", input_sel, has_input, (time.time()-t0)*1000)

        if not has_input:
            log.warning("[verify_ready] input not found, reloading and re-discovering")
            self._invalidate_sel_cache()
            self._tab.navigate(cfg["url"], wait_s=4.0)
            sels = self._discover_selectors()
            input_sel = sels["input"]
            has_input = self._js(f"return !!document.querySelector({json.dumps(input_sel)})")
            log.info("[verify_ready] after reload: input_sel=%s found=%s", input_sel, has_input)
        if not has_input:
            self._last_error = json.dumps({"status": "error", "error": "no_input_box",
                "suggestion": "Chat input not found after reload. Try again."})
            return False

        log.info("[verify_ready] OK (%.0fms)", (time.time()-t0)*1000)
        return True

    # ── Step 3: type message and send ─────────────────────────────────────────

    def send(self, msg: str) -> bool:
        import time
        cfg = self._cfg
        log = self._log
        t0 = time.time()
        sels = self._discover_selectors()
        input_sel = sels["input"]
        send_sel = sels["send_btn"]
        asst_sel = cfg["assistant"]
        user_sel = cfg.get("user")
        log.info("[send] using input=%s send=%s", input_sel, send_sel)

        # Register network watcher BEFORE sending — catches /backend-api/f/conversation
        # Pattern broad enough to match the stream URL, filtering in wait_response()
        self._net_watcher_q = self._tab.watch_requests("backend-api")
        log.info("[send] network watcher registered for backend-api requests")

        # Snapshot before send
        self._msg_count_before = int(self._js(
            f"return document.querySelectorAll({json.dumps(asst_sel)}).length"
        ) or 0)
        self._last_text_before = self._js(
            f"const m=document.querySelectorAll({json.dumps(asst_sel)});"
            f"return m.length?m[m.length-1].innerText?.substring(0,200):''"
        ) or ""
        log.info("[send] snapshot: %d assistant msgs, last=%r", self._msg_count_before, self._last_text_before[:40])

        # Focus: CDP mouse click (real browser-level focus, more reliable than JS .focus())
        rect_json = self._js(f"""
            const el = document.querySelector({json.dumps(input_sel)});
            const r = el?.getBoundingClientRect();
            return r ? JSON.stringify({{x: Math.round(r.left+r.width/2), y: Math.round(r.top+r.height/2)}}) : null
        """)
        if rect_json:
            try:
                coords = json.loads(rect_json)
                cx, cy = coords["x"], coords["y"]
                self._cdp("Input.dispatchMouseEvent", {"type": "mousePressed", "x": cx, "y": cy, "button": "left", "clickCount": 1})
                self._cdp("Input.dispatchMouseEvent", {"type": "mouseReleased", "x": cx, "y": cy, "button": "left", "clickCount": 1})
                log.info("[send] CDP mouse click at (%d,%d) (%.0fms)", cx, cy, (time.time()-t0)*1000)
                time.sleep(0.1)
            except Exception as e:
                log.warning("[send] CDP click failed (%s), JS focus fallback", e)
                self._js(f"const el=document.querySelector({json.dumps(input_sel)});if(el){{el.focus();el.click()}}")
        else:
            log.warning("[send] no rect for %s, JS focus fallback", input_sel)
            self._js(f"const el=document.querySelector({json.dumps(input_sel)});if(el){{el.focus();el.click()}}")

        time.sleep(0.05)

        # Ctrl+A to clear existing content
        self._cdp("Input.dispatchKeyEvent", {"type": "keyDown", "modifiers": 2, "key": "a", "code": "KeyA", "windowsVirtualKeyCode": 65})
        self._cdp("Input.dispatchKeyEvent", {"type": "keyUp",   "modifiers": 2, "key": "a", "code": "KeyA", "windowsVirtualKeyCode": 65})
        time.sleep(0.05)
        log.info("[send] ctrl+a done, pasting msg len=%d (%.0fms)", len(msg), (time.time()-t0)*1000)

        # Primary input: ClipboardEvent paste — ProseMirror handles this natively
        self._js(f"""
            const dt = new DataTransfer();
            dt.setData('text/plain', {json.dumps(msg)});
            const el = document.activeElement;
            if (el) el.dispatchEvent(new ClipboardEvent('paste', {{clipboardData: dt, bubbles: true}}));
        """)
        time.sleep(0.2)

        content = self._js(f"return document.querySelector({json.dumps(input_sel)})?.innerText||''") or ""
        log.info("[send] content after paste: len=%d preview=%r (%.0fms)", len(content), content[:40], (time.time()-t0)*1000)

        # Fallback: insertText (if paste didn't land)
        if not content or msg[:15] not in content:
            log.info("[send] ClipboardEvent missed, trying insertText fallback")
            self._cdp("Input.insertText", {"text": msg})
            time.sleep(0.2)
            content = self._js(f"return document.querySelector({json.dumps(input_sel)})?.innerText||''") or ""
            log.info("[send] content after insertText: len=%d preview=%r (%.0fms)", len(content), content[:40], (time.time()-t0)*1000)

        if not content or len(content) < len(msg) // 2:
            log.error("textarea empty after paste+insertText. content=%r", content[:80])
            self._last_error = json.dumps({"status": "error", "error": "textarea empty after input"})
            self._send_verified = False
            return False

        # Count user messages before firing send
        user_count_before = 0
        if user_sel:
            user_count_before = int(self._js(f"return document.querySelectorAll({json.dumps(user_sel)}).length") or 0)
        log.info("[send] firing: Enter + btn click (user_msgs_before=%d) (%.0fms)", user_count_before, (time.time()-t0)*1000)

        # Send: Enter key ONLY — button click after Enter hits stop-button (visible after ~100ms)
        # which aborts the stream. Enter alone is sufficient for ProseMirror to submit.
        self._cdp("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
        self._cdp("Input.dispatchKeyEvent", {"type": "keyUp",   "key": "Enter", "code": "Enter"})
        log.info("[send] Enter key fired (no button click)")

        # Verify: user message appeared in DOM or stop button shown
        stop_sel = cfg.get("stop_btn")
        sent = False
        for i in range(6):
            time.sleep(0.5)
            if stop_sel and self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"):
                log.info("[send] stop_btn appeared at check %d (%.0fms) — send confirmed", i, (time.time()-t0)*1000)
                sent = True
                break
            if user_sel:
                uc = int(self._js(f"return document.querySelectorAll({json.dumps(user_sel)}).length") or 0)
                if uc > user_count_before:
                    log.info("[send] user msg in DOM at check %d (%.0fms) — send confirmed", i, (time.time()-t0)*1000)
                    sent = True
                    break

        self._send_verified = sent
        if sent:
            cur_url = self._js("return location.href") or ""
            if "/c/" in cur_url:
                self.conv_url = cur_url.split("?")[0]
                log.info("[send] conversation anchored: %s", self.conv_url)
            log.info("[send] OK (%d chars, %.0fms total)", len(msg), (time.time()-t0)*1000)
        else:
            log.warning("[send] FAIL — message not confirmed in DOM after 3s (%.0fms)", (time.time()-t0)*1000)
            self._last_error = json.dumps({"status": "error", "error": "message typed but not confirmed sent (not in DOM after 3s)"})
        return sent

    # ── MutationObserver watcher ──────────────────────────────────────────────

    def _inject_watcher(self) -> None:
        """Inject a MutationObserver into the page that tracks GPT state in
        window.__gptWatcher — stop button visibility, last text, hash stability,
        and error banners. Avoids heavy DOM polling in wait_response()."""
        cfg = self._cfg
        asst_sel = cfg["assistant"]
        stop_sel = cfg.get("stop_btn") or ""
        log = self._log

        js = f"""
        (function() {{
            if (window.__gptWatcherObs) {{ window.__gptWatcherObs.disconnect(); }}
            window.__gptWatcher = {{
                stopVisible: !!({('document.querySelector(' + repr(stop_sel) + ')') if stop_sel else 'null'}),
                lastMsgCount: document.querySelectorAll({repr(asst_sel)}).length,
                lastText: '',
                lastHash: 0,
                stableCount: 0,
                streaming: false,
                done: false,
                errors: [],
                ts: Date.now()
            }};
            function _hash(s) {{
                let h = 0;
                const n = Math.min(s.length, 500);
                for (let i = 0; i < n; i++) {{ h = (Math.imul(31, h) + s.charCodeAt(i)) | 0; }}
                return h;
            }}
            function _readLast() {{
                const msgs = document.querySelectorAll({repr(asst_sel)});
                if (!msgs.length) return '';
                for (let i = msgs.length - 1; i >= 0; i--) {{
                    const el = msgs[i];
                    const prose = el.querySelector('.prose, .markdown');
                    if (prose) {{
                        const clone = prose.cloneNode(true);
                        clone.querySelectorAll('.puik-root,.not-prose,.not-markdown,button,[role=button]').forEach(e => e.remove());
                        const pt = (clone.innerText || '').trim();
                        if (pt.length > 0) return pt.substring(0, 50000);
                    }}
                    const tas = el.querySelectorAll('textarea');
                    if (tas.length) {{
                        const val = [...tas].map(t => (t.value || t.textContent || '').trim()).filter(s => s.length > 0).join(' ').trim();
                        if (val.length > 0) return val.substring(0, 50000);
                    }}
                    const it = (el.innerText || '').trim();
                    if (it.length > 0) return it.substring(0, 50000);
                }}
                return '';
            }}
            function _update() {{
                const w = window.__gptWatcher;
                const stop = {('!!document.querySelector(' + repr(stop_sel) + ')') if stop_sel else 'false'};
                const msgs = document.querySelectorAll({repr(asst_sel)});
                w.stopVisible = stop;
                w.lastMsgCount = msgs.length;
                if (stop) w.streaming = true;
                const text = _readLast();
                if (text) {{
                    const h = _hash(text);
                    if (h !== w.lastHash) {{ w.stableCount = 0; w.lastHash = h; }}
                    else {{ w.stableCount++; }}
                    w.lastText = text;
                }}
                if (w.streaming && !stop && text) w.done = true;
                // Error banner detection
                const errSels = ['[class*="error-message"]','[data-testid*="error"]','.text-red-500:not(button)','[role="alert"]'];
                for (const sel of errSels) {{
                    const el = document.querySelector(sel);
                    if (el) {{
                        const t = (el.innerText || '').trim();
                        if (t && t.length > 3 && !w.errors.includes(t)) w.errors.push(t.substring(0,200));
                    }}
                }}
                w.ts = Date.now();
            }}
            const obs = new MutationObserver(_update);
            obs.observe(document.body, {{ childList: true, subtree: true, characterData: true }});
            window.__gptWatcherObs = obs;
            _update();
        }})();
        """
        try:
            self._js(js)
            log.info("[watcher] MutationObserver injected")
        except Exception as e:
            log.warning("[watcher] inject failed: %s", e)

    def _watcher_state(self) -> dict:
        """Read current watcher state from page. Returns empty dict on error."""
        try:
            raw = self._js("return window.__gptWatcher ? JSON.stringify(window.__gptWatcher) : null")
            return json.loads(raw) if raw else {}
        except Exception:
            return {}

    # ── Step 4: wait for complete response ────────────────────────────────────

    def wait_response(self, timeout_s: int = 150) -> str:
        """
        Wait for the ChatGPT assistant response.

        Uses MutationObserver (window.__gptWatcher) for event-driven detection:
          1. Wait for page navigation / stream start (watcher: stopVisible or msgCount change)
          2. Wait for readyState=complete
          3. Wait for stop button to appear  (watcher: stopVisible=true or early response)
          4. Wait for stop button to go away (watcher: done=true)
          5. Stability window: same text hash for STABLE_NEEDED consecutive observer ticks

        Falls back to direct DOM polling if watcher unavailable.
        """
        import time
        cfg = self._cfg
        log = self._log

        if not self._send_verified:
            return self._last_error or json.dumps({"status": "error", "error": "send not verified"})

        asst_sel = cfg["assistant"]
        stop_sel = cfg.get("stop_btn") or ""
        t0 = time.time()
        STABLE_NEEDED = 2   # same hash N times = response stable

        # Drain the net watcher (not needed for DOM/observer polling)
        if self._net_watcher_q:
            try:
                self._tab.unwatch_requests(self._net_watcher_q)
            except Exception:
                pass
            self._net_watcher_q = None

        # Inject MutationObserver so phases 3+4 read lightweight watcher state
        self._inject_watcher()

        def _elapsed_ms() -> int:
            return int((time.time() - t0) * 1000)

        def _read_last() -> str:
            return self._js(f"""
                const msgs = document.querySelectorAll({json.dumps(asst_sel)});
                if (!msgs.length) return '';
                for (let i = msgs.length - 1; i >= 0; i--) {{
                    const el = msgs[i];
                    const prose = el.querySelector('.prose, .markdown');
                    if (prose) {{
                        const clone = prose.cloneNode(true);
                        clone.querySelectorAll('.puik-root, .not-prose, .not-markdown, button, [role=button]')
                             .forEach(e => e.remove());
                        const pt = (clone.innerText || '').trim();
                        if (pt.length > 0) return pt.substring(0, 50000);
                    }}
                    const tas = el.querySelectorAll('textarea');
                    if (tas.length) {{
                        const val = [...tas].map(t => (t.value || t.textContent || '').trim())
                                           .filter(s => s.length > 0).join(' ').trim();
                        if (val.length > 0) return val.substring(0, 50000);
                    }}
                    const it = (el.innerText || '').trim();
                    if (it.length > 0) return it.substring(0, 50000);
                }}
                return '';
            """) or ""

        def _stop_visible() -> bool:
            if not stop_sel:
                return False
            return bool(self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"))

        def _page_state() -> dict:
            return json.loads(self._js(f"""
                return JSON.stringify({{
                    url: location.href,
                    ready: document.readyState,
                    asst: document.querySelectorAll({json.dumps(asst_sel)}).length,
                    stop: {'!!document.querySelector(' + json.dumps(stop_sel) + ')' if stop_sel else 'false'},
                    title: document.title.substring(0,40)
                }});
            """) or '{}')

        def _classify_error(errors: list) -> str:
            if not errors:
                return "unknown"
            text = " ".join(errors).lower()
            if any(k in text for k in ("rate limit", "too many", "slow down")):
                return "rate_limit"
            if any(k in text for k in ("log in", "sign in", "auth", "session")):
                return "auth"
            if any(k in text for k in ("network", "connection", "offline")):
                return "network"
            if any(k in text for k in ("unavailable", "overloaded", "capacity")):
                return "model_unavailable"
            return "ui_error"

        def _anchor_url():
            cur_url = self._js("return location.href") or ""
            if "/c/" in cur_url:
                self.conv_url = cur_url.split("?")[0]

        # ── Phase 1: wait for navigation / stream start (max 15s) ──────────
        log.info("[wait] phase1: waiting for nav to /c/ or stop button (%dms)", _elapsed_ms())
        nav_deadline = t0 + 15.0
        nav_done = False
        while time.time() < nav_deadline:
            try:
                st = _page_state()
            except Exception:
                time.sleep(0.3)
                continue
            url = st.get("url", "")
            stop = st.get("stop", False)
            asst = st.get("asst", 0)
            log.debug("[wait] p1 url=%s stop=%s asst=%d (%dms)", url[:60], stop, asst, _elapsed_ms())
            if "/c/" in url or stop or asst > self._msg_count_before:
                log.info("[wait] phase1 OK: url=%s stop=%s asst=%d (%dms)", url[:60], stop, asst, _elapsed_ms())
                nav_done = True
                break
            time.sleep(0.3)

        if not nav_done:
            log.warning("[wait] phase1 timeout — no nav/stop after 15s. Checking DOM anyway.")
            text = _read_last()
            if text:
                log.info("[wait] phase1 fallback: found text (%d chars, %dms)", len(text), _elapsed_ms())
                _anchor_url()
                return text

        # ── Phase 2: wait for readyState=complete (new page) ──────────────
        log.info("[wait] phase2: waiting for readyState=complete (%dms)", _elapsed_ms())
        rdy_deadline = t0 + 20.0
        while time.time() < rdy_deadline:
            try:
                ready = self._js("return document.readyState")
                if ready == "complete":
                    log.info("[wait] phase2 readyState=complete (%dms)", _elapsed_ms())
                    break
            except Exception:
                pass
            time.sleep(0.3)

        # ── Phase 3: wait for stop button / stream start (max 30s) ────────
        log.info("[wait] phase3: waiting for stop button (%dms)", _elapsed_ms())
        stop_deadline = t0 + 30.0
        stop_appeared = False
        while time.time() < stop_deadline:
            try:
                w = self._watcher_state()
                if w:
                    stop = w.get("stopVisible", False)
                    asst = w.get("lastMsgCount", 0)
                    errs = w.get("errors", [])
                    if errs:
                        kind = _classify_error(errs)
                        log.warning("[wait] p3 error banner: %s — %s", kind, errs[0])
                        return json.dumps({"status": "error", "error": kind, "detail": errs[0]})
                else:
                    stop = _stop_visible()
                    asst = int(self._js(f"return document.querySelectorAll({json.dumps(asst_sel)}).length") or 0)
                log.debug("[wait] p3 stop=%s asst=%d (%dms)", stop, asst, _elapsed_ms())
                if stop:
                    log.info("[wait] phase3 stop_btn appeared (%dms)", _elapsed_ms())
                    stop_appeared = True
                    break
                if asst > self._msg_count_before:
                    # Fast / cached reply — already done
                    text = (w.get("lastText") or "") if w else _read_last()
                    if not text:
                        text = _read_last()
                    if text:
                        log.info("[wait] phase3 early response (%d chars, %dms)", len(text), _elapsed_ms())
                        _anchor_url()
                        return text
            except Exception as e:
                log.debug("[wait] p3 err: %s", e)
            time.sleep(0.3)

        if not stop_appeared:
            log.warning("[wait] phase3: stop btn never appeared. Checking DOM.")
            text = _read_last()
            if text:
                log.info("[wait] phase3 fallback text (%d chars)", len(text))
                return text

        # ── Phase 4: wait for stop button gone + stability window ─────────
        log.info("[wait] phase4: waiting for done + stability (%dms)", _elapsed_ms())
        done_deadline = t0 + timeout_s
        stable_hits = 0
        last_hash = 0
        while time.time() < done_deadline:
            try:
                w = self._watcher_state()
                if w:
                    stop = w.get("stopVisible", False)
                    asst_count = w.get("lastMsgCount", 0)
                    watcher_text = w.get("lastText", "")
                    watcher_hash = w.get("lastHash", 0)
                    stable_count = w.get("stableCount", 0)
                    errs = w.get("errors", [])

                    if errs:
                        kind = _classify_error(errs)
                        log.warning("[wait] p4 error banner: %s — %s", kind, errs[0])
                        return json.dumps({"status": "error", "error": kind, "detail": errs[0]})

                    log.debug("[wait] p4 stop=%s asst=%d stable=%d hash=%d (%dms)",
                              stop, asst_count, stable_count, watcher_hash, _elapsed_ms())

                    if not stop and asst_count > self._msg_count_before and watcher_text:
                        # Track hash stability across our own polling ticks too
                        if watcher_hash != last_hash:
                            last_hash = watcher_hash
                            stable_hits = 0
                        else:
                            stable_hits += 1

                        if stable_hits >= STABLE_NEEDED or stable_count >= STABLE_NEEDED:
                            elapsed = round(time.time()-t0, 1)
                            log.info("[wait] DONE: stable (self=%d obs=%d) %d chars, %.1fs",
                                     stable_hits, stable_count, len(watcher_text), elapsed)
                            _anchor_url()
                            return watcher_text
                else:
                    # Watcher unavailable — direct DOM poll
                    stop = _stop_visible()
                    asst_count = int(self._js(f"return document.querySelectorAll({json.dumps(asst_sel)}).length") or 0)
                    if not stop and asst_count > self._msg_count_before:
                        text = _read_last()
                        if text:
                            elapsed = round(time.time()-t0, 1)
                            log.info("[wait] DONE (no watcher): %d chars, %.1fs", len(text), elapsed)
                            _anchor_url()
                            return text
            except Exception as e:
                log.debug("[wait] p4 err: %s", e)
            time.sleep(0.5)

        # Final fallback: whatever is in DOM right now
        elapsed = round(time.time()-t0, 1)
        log.warning("[wait] timeout after %.1fs, final DOM read", elapsed)
        text = _read_last()
        if text:
            _anchor_url()
            return text
        return json.dumps({"status": "error", "error": f"no_response after {timeout_s}s"})

    # ── read_last helper ──────────────────────────────────────────────────────

    def read_last(self) -> str:
        import time
        cfg = self._cfg
        asst_sel = cfg["assistant"]
        stop_sel = cfg.get("stop_btn")
        for _ in range(60):
            text = self._js(f"""
                const msgs = document.querySelectorAll({json.dumps(asst_sel)});
                if (!msgs.length) return '';
                const last = msgs[msgs.length-1];
                const clone = last.cloneNode(true);
                clone.querySelectorAll('button,[role=button]').forEach(e=>e.remove());
                return clone.innerText?.trim().substring(0,50000)||'';
            """) or ""
            streaming = bool(stop_sel and self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"))
            if text and len(text) > 2 and not streaming:
                return text
            if text and len(text) > 2:
                time.sleep(0.5)
                continue
            if not streaming:
                break
            time.sleep(0.5)
        result = self._js(f"""
            const msgs = document.querySelectorAll({json.dumps(asst_sel)});
            if (!msgs.length) return '';
            return msgs[msgs.length-1].innerText?.trim().substring(0,50000)||'';
        """) or ""
        return result if result else "No messages"

    # ── is_streaming helper ───────────────────────────────────────────────────

    def is_streaming_state(self) -> str:
        cfg = self._cfg
        asst_sel = cfg["assistant"]
        stop_sel = cfg.get("stop_btn")
        streaming = bool(stop_sel and self._js(f"return !!document.querySelector({json.dumps(stop_sel)})"))
        chars = len(self._js(f"""
            const msgs = document.querySelectorAll({json.dumps(asst_sel)});
            return msgs.length ? msgs[msgs.length-1].innerText?.trim()||'' : '';
        """) or "")
        state = ("thinking" if (streaming and chars == 0) else
                 "generating" if streaming else
                 "complete" if chars > 0 else "idle")
        return json.dumps({"state": state, "streaming": streaming, "chars": chars, "open": True})

    # ── history helper ────────────────────────────────────────────────────────

    def history(self) -> str:
        cfg = self._cfg
        asst_sel = cfg["assistant"]
        user_sel = cfg.get("user")
        if user_sel:
            msgs_json = self._js(f"""
                const m = [];
                document.querySelectorAll('[data-message-author-role]').forEach(e => {{
                    const r = e.getAttribute('data-message-author-role');
                    const t = e.innerText?.trim()?.substring(0,300);
                    if (t) m.push({{role: r, text: t}});
                }});
                return JSON.stringify(m.slice(-5));
            """) or "[]"
        else:
            msgs_json = "[]"
        try:
            msgs = json.loads(msgs_json)
            return "\n".join(f'> {"YOU" if m["role"]=="user" else self.platform.upper()}: {m["text"][:200]}' for m in msgs)
        except Exception:
            return msgs_json or "No messages"

    # ── Full pipeline ─────────────────────────────────────────────────────────

    def run(self, msg: str, wait: bool = True) -> str:
        import time
        log = self._log
        with self._lock:
            for attempt in range(2):
                try:
                    if not self.ensure():
                        return self._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
                    if not self.verify_ready():
                        if attempt == 0:
                            log.info("not ready, retry %d", attempt + 1)
                            continue
                        return self._last_error or json.dumps({"status": "error", "error": "not_ready"})
                    if not self.send(msg):
                        if attempt == 0:
                            log.info("send failed, retry %d", attempt + 1)
                            continue
                        return self._last_error or json.dumps({"status": "error", "error": "send_failed"})
                    if not wait:
                        return json.dumps({"status": "sent", "chars": len(msg),
                            "message": "Message sent. Use action=read_last to get response."})
                    return self.wait_response()
                except Exception as e:
                    log.exception("pipeline error attempt %d", attempt)
                    if attempt == 0:
                        time.sleep(1)
                    else:
                        return json.dumps({"status": "error", "error": str(e)})
        return json.dumps({"status": "error", "error": "no_response"})


def _get_chat_pipeline(platform: str) -> "ChatPipeline":
    global _chat_pipelines
    if platform not in _chat_pipelines:
        _chat_pipelines[platform] = ChatPipeline(platform)
    return _chat_pipelines[platform]


# ---------------------------------------------------------------------------
# Tool dispatch
# ---------------------------------------------------------------------------


def _handle_chat(platform: str, args: dict) -> str:
    """Thin shim — routes to ChatPipeline. Ensures tab exists first."""
    p = _get_chat_pipeline(platform)
    # ChatPipeline needs a tab to operate on. Ensure browser is up.
    _get_browser()
    action = args.get("action", "send")

    if action == "send":
        msg = args.get("message", "")
        if not msg:
            return json.dumps({"error": "message required"})
        if len(msg) > 32000:
            return json.dumps({"error": f"message too long ({len(msg)} chars, max 32000)"})
        return p.run(msg, wait=args.get("wait", True))

    elif action == "read_last":
        if not p._tab:
            if not p.ensure():
                return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        return p.read_last()

    elif action == "is_streaming":
        if not p._tab:
            if not p.ensure():
                return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        return p.is_streaming_state()

    elif action == "history":
        if not p._tab:
            if not p.ensure():
                return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        return p.history()

    elif action == "check_session":
        if not p.ensure():
            return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        authenticated = p._js("""
            const text = document.body?.innerText || '';
            const hasLoginBtn = !!document.querySelector('[data-testid="login-button"], a[href*="/auth/login"]');
            const loginSignals = ['log in', 'sign in', 'sign up', 'create account', 'iniciar sesión'];
            const lowerText = text.toLowerCase();
            return JSON.stringify({
                authenticated: !hasLoginBtn && !loginSignals.some(s => lowerText.includes(s)) && text.length > 100,
                url: location.href
            });
        """) or "{}"
        try:
            info = json.loads(authenticated)
        except Exception:
            info = {}
        return json.dumps({"check": "session", **info})

    elif action == "debug_network":
        # Dump recent network requests so we can find the real conversation API URL
        if not p._tab:
            if not p.ensure():
                return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        reqs = p._tab.get_network_requests()
        # Show last 30, filter to just URLs + status
        entries = [{"url": r["url"][:120], "method": r.get("method","?"), "status": r.get("status")}
                   for r in reqs[-30:]]
        return json.dumps({"network_requests": entries, "total": len(reqs)}, indent=2)

    elif action == "debug_watch":
        # Watch ALL backend-api network events for 20s and return them raw.
        # Shows EXACTLY what the watcher sees: req_id, url, event type.
        import time as _t
        if not p._tab:
            if not p.ensure():
                return p._last_error or json.dumps({"status": "error", "error": "platform_unavailable"})
        q = p._tab.watch_requests("backend-api")
        events = []
        deadline = _t.time() + 20.0
        try:
            while _t.time() < deadline:
                remaining = deadline - _t.time()
                try:
                    evt = q.get(timeout=min(remaining, 2.0))
                    event, req_id, url, data = evt
                    events.append({"event": event, "req_id": req_id[:8], "url": url[:120], "data": str(data)[:30] if data else None})
                except Exception:
                    pass  # queue.Empty — keep waiting
        finally:
            p._tab.unwatch_requests(q)
        return json.dumps({"watched_events": events, "count": len(events)}, indent=2)

    return json.dumps({"error": f"unknown action: {action}"})


def dispatch_tool(name: str, args: dict) -> Any:
    b = _get_browser()

    if name == "navigate":
        url = args["url"]
        wait_s = float(args.get("wait_s", 3.0))
        tab = _get_tab(url, wait_s=wait_s)
        return f"Navigated to {tab.current_url()}"

    elif name == "screenshot":
        tab = _get_tab()
        fmt = args.get("format", "png")
        quality = int(args.get("quality", 80))
        b64 = tab.screenshot_base64(format=fmt, quality=quality)
        return json.dumps({"format": fmt, "data": b64})

    elif name == "read":
        tab = _get_tab()
        selector = args.get("selector", "body")
        text = tab.js(f"return document.querySelector({json.dumps(selector)})?.innerText?.trim() || ''")
        return text or "(empty)"

    elif name == "find":
        tab = _get_tab()
        intent = args["intent"]
        from tools.v4.page_analyzer import FormFinder
        finder = FormFinder(tab)
        result = finder.find(intent)
        if result is None:
            return json.dumps({"found": False, "backend_node_id": None})
        return json.dumps({"found": True, **result.to_dict()})

    elif name == "click":
        tab = _get_tab()
        node_id = args.get("backend_node_id")
        selector = args.get("selector")
        if node_id is not None:
            result = tab.send("DOM.resolveNode", {"backendNodeId": int(node_id)})
            obj_id = result.get("object", {}).get("objectId")
            if obj_id:
                tab.send("Runtime.callFunctionOn", {
                    "objectId": obj_id,
                    "functionDeclaration": "function(){this.click()}",
                    "returnByValue": True,
                })
                return f"Clicked node {node_id}"
            return f"Node {node_id} not found in DOM"
        elif selector:
            ok = tab.click(selector)
            return f"Clicked {selector}" if ok else f"Selector not found: {selector}"
        return "Error: provide backend_node_id or selector"

    elif name == "type":
        tab = _get_tab()
        text = args["text"]
        tab.send("Input.insertText", {"text": text})
        return f"Typed {len(text)} chars"

    elif name == "console_logs":
        tab = _get_tab()
        if not tab._console_enabled:
            tab.enable_console()
        logs = b.console_logs(tab)
        level_filter = args.get("level")
        if level_filter:
            logs = [l for l in logs if l.get("level") == level_filter]
        limit = int(args.get("limit", 50))
        return json.dumps(logs[-limit:])

    elif name == "network_log":
        tab = _get_tab()
        if not tab._network_enabled:
            tab.enable_network()
        reqs = b.network_log(tab)
        pattern = args.get("url_pattern")
        if pattern:
            reqs = [r for r in reqs if pattern in r.get("url", "")]
        limit = int(args.get("limit", 50))
        return json.dumps(reqs[-limit:])

    elif name == "metrics":
        tab = _get_tab()
        m = b.metrics(tab)
        key = args.get("key")
        if key:
            return json.dumps({key: m.get(key)})
        return json.dumps(m)

    elif name == "save_cookies":
        tab = _get_tab()
        b.save_cookies(tab)
        return "Cookies saved"

    elif name == "restore_cookies":
        tab = _get_tab()
        count = b.restore_cookies(tab)
        return f"Restored {count} cookies"

    elif name == "save_session":
        tab = _get_tab()
        stats = b.save_session(tab)
        return json.dumps(stats)

    elif name == "session_info":
        return json.dumps(b.session_info())

    elif name == "record_task":
        domain = args["domain"]
        task_name = args["task_name"]
        b.record_task(domain, task_name)
        return f"Recording started: {domain}/{task_name}"

    elif name == "stop_recording":
        steps = b.stop_recording()
        return json.dumps({"steps": len(steps), "saved": len(steps) > 0})

    elif name == "replay":
        tab = _get_tab()
        domain = args["domain"]
        task_name = args["task_name"]
        ok, first_fail = b.replay(tab, domain, task_name)
        return json.dumps({"ok": ok, "first_failed_step": first_fail})

    elif name == "scroll":
        import time as _time
        tab = _get_tab()
        direction = args.get("direction", "down")
        amount = int(args.get("amount", 500))
        if direction == "top":
            tab.js("window.scrollTo(0, 0)")
        elif direction == "bottom":
            tab.js("window.scrollTo(0, document.body.scrollHeight)")
        elif direction == "up":
            tab.js(f"window.scrollBy(0, -{amount})")
        else:
            tab.js(f"window.scrollBy(0, {amount})")
        _time.sleep(0.3)
        pos = tab.js("return window.scrollY") or 0
        return json.dumps({"scrolled": direction, "amount": amount, "scrollY": pos})

    elif name == "wait":
        import time as _time
        tab = _get_tab()
        ms = int(args.get("ms", 1000))
        selector = args.get("selector")
        if selector:
            deadline = _time.time() + ms / 1000
            found = False
            while _time.time() < deadline:
                count = tab.js(f"return document.querySelectorAll({json.dumps(selector)}).length") or 0
                if count > 0:
                    found = True
                    break
                _time.sleep(0.2)
            return json.dumps({"found": found, "selector": selector, "waited_ms": ms})
        else:
            _time.sleep(ms / 1000)
            return f"Waited {ms}ms"

    elif name == "js":
        tab = _get_tab()
        code = args["code"]
        # tab.js() already wraps in IIFE when "return " is present — pass code directly
        result = tab.js(code)
        return json.dumps(result) if not isinstance(result, str) else result

    elif name in ("gpt", "grok"):
        return _handle_chat(name, args)

    elif name == "page_info":
        tab = _get_tab()
        info = tab.js('''
            var els = document.querySelectorAll('a,button,input,select,textarea,[role=button],[role=link]');
            var forms = document.querySelectorAll('form');
            var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
                var s = window.getComputedStyle(e);
                return (s.position === 'fixed' || s.position === 'sticky') &&
                       parseInt(s.zIndex) > 100 && e.offsetHeight > 50;
            });
            return JSON.stringify({
                url: location.href,
                title: document.title,
                interactive: els.length,
                forms: forms.length,
                has_overlay: overlays.length > 0,
                overlay_count: overlays.length
            });
        ''') or '{}'
        return info

    elif name == "status":
        import subprocess as _sp
        tab = _get_tab()
        url = tab.js("return location.href") or "unknown"
        title = tab.js("return document.title") or ""
        try:
            pid = _sp.check_output(["pgrep", "-f", "chrome.*remote-debugging"], text=True).strip().split("\n")[0]
        except Exception:
            pid = "unknown"
        return json.dumps({"url": url, "title": title, "chrome_pid": pid})

    elif name == "analyze":
        tab = _get_tab()
        result = tab.js('''
            var forms = Array.from(document.querySelectorAll('form')).map(function(f, fi) {
                var fields = Array.from(f.querySelectorAll('input,select,textarea')).map(function(el) {
                    var label = '';
                    if (el.id) { var l = document.querySelector('label[for="'+el.id+'"]'); if(l) label = l.textContent.trim(); }
                    if (!label) label = el.placeholder || el.name || el.type || '';
                    return {tag: el.tagName.toLowerCase(), type: el.type||'', name: el.name||'', id: el.id||'', label: label, value: el.value||''};
                });
                return {index: fi, action: f.action||'', method: f.method||'get', fields: fields};
            });
            var buttons = Array.from(document.querySelectorAll('button,[role=button],input[type=submit],input[type=button]')).slice(0,20).map(function(b) {
                return {tag: b.tagName.toLowerCase(), text: (b.textContent||b.value||'').trim().slice(0,60), type: b.type||''};
            });
            var overlays = Array.from(document.querySelectorAll('*')).filter(function(e) {
                var s = window.getComputedStyle(e);
                return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex)>100 && e.offsetHeight>50;
            }).slice(0,5).map(function(e){ return {tag: e.tagName.toLowerCase(), id: e.id||'', cls: e.className.toString().slice(0,60)}; });
            var active = document.activeElement ? {tag: document.activeElement.tagName.toLowerCase(), id: document.activeElement.id||''} : null;
            return JSON.stringify({forms: forms, buttons: buttons, overlays: overlays, active_element: active});
        ''') or '{}'
        return result

    elif name == "fill":
        tab = _get_tab()
        selector = args["selector"]
        value = args["value"]
        result = tab.js('''
            return (function() {
                var el = document.querySelector(''' + json.dumps(selector) + ''');
                if (!el) return JSON.stringify({ok: false, error: "selector not found"});
                var tag = el.tagName.toLowerCase();
                var type = (el.type || '').toLowerCase();
                if (tag === 'select') {
                    el.value = ''' + json.dumps(value) + ''';
                    el.dispatchEvent(new Event('change', {bubbles: true}));
                } else if (type === 'checkbox' || type === 'radio') {
                    var check = ''' + json.dumps(value) + ''' === 'true' || ''' + json.dumps(value) + ''' === true;
                    el.checked = check;
                    el.dispatchEvent(new Event('change', {bubbles: true}));
                } else {
                    var nativeInputValueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value') ||
                                                 Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
                    if (nativeInputValueSetter && nativeInputValueSetter.set) {
                        nativeInputValueSetter.set.call(el, ''' + json.dumps(value) + ''');
                    } else {
                        el.value = ''' + json.dumps(value) + ''';
                    }
                    el.dispatchEvent(new Event('input', {bubbles: true}));
                    el.dispatchEvent(new Event('change', {bubbles: true}));
                }
                return JSON.stringify({ok: true, tag: tag, type: type, value: el.value});
            })()
        ''') or '{"ok": false, "error": "js returned null"}'
        return result

    elif name == "form_fill":
        import time as _time
        tab = _get_tab()
        fields = args["fields"]
        form_index = int(args.get("form_index", 0))
        results = {}
        for label, value in fields.items():
            label_js = json.dumps(label)
            value_js = json.dumps(value)
            res = tab.js(f'''
                return (function() {{
                    var forms = document.querySelectorAll('form');
                    var form = forms[{form_index}] || document;
                    var inputs = Array.from(form.querySelectorAll('input,select,textarea'));
                    var target = null;
                    var lq = {label_js}.toLowerCase();
                    for (var i=0; i<inputs.length; i++) {{
                        var el = inputs[i];
                        var candidates = [el.name, el.id, el.placeholder, el.getAttribute('aria-label')];
                        var lbl = '';
                        if (el.id) {{ var l = document.querySelector('label[for="'+el.id+'"]'); if(l) lbl = l.textContent; }}
                        candidates.push(lbl);
                        for (var j=0; j<candidates.length; j++) {{
                            if (candidates[j] && candidates[j].toLowerCase().indexOf(lq) !== -1) {{ target = el; break; }}
                        }}
                        if (target) break;
                    }}
                    if (!target) return JSON.stringify({{ok: false, error: 'field not found: '+{label_js}}});
                    var tag = target.tagName.toLowerCase();
                    var type = (target.type||'').toLowerCase();
                    if (tag === 'select') {{
                        target.value = {value_js};
                        target.dispatchEvent(new Event('change', {{bubbles: true}}));
                    }} else if (type === 'checkbox' || type === 'radio') {{
                        target.checked = ({value_js} === 'true' || {value_js} === true);
                        target.dispatchEvent(new Event('change', {{bubbles: true}}));
                    }} else {{
                        var proto = tag === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
                        var setter = Object.getOwnPropertyDescriptor(proto, 'value');
                        if (setter && setter.set) {{ setter.set.call(target, {value_js}); }}
                        else {{ target.value = {value_js}; }}
                        target.dispatchEvent(new Event('input', {{bubbles: true}}));
                        target.dispatchEvent(new Event('change', {{bubbles: true}}));
                    }}
                    return JSON.stringify({{ok: true, field: {label_js}, value: target.value}});
                }})()
            ''') or f'{{"ok": false, "error": "js null for {label}"}}'
            results[label] = json.loads(res) if res else {"ok": False}
            _time.sleep(0.1)
        return json.dumps({"filled": results})

    elif name == "submit":
        tab = _get_tab()
        selector = args.get("selector")
        if selector:
            result = tab.js(f'''
                return (function() {{
                    var el = document.querySelector({json.dumps(selector)});
                    if (!el) return JSON.stringify({{ok: false, error: "selector not found"}});
                    el.click();
                    return JSON.stringify({{ok: true, clicked: {json.dumps(selector)}}});
                }})()
            ''') or '{"ok": false}'
        else:
            result = tab.js('''
                return (function() {
                    var btn = document.querySelector('button[type=submit],input[type=submit],[role=button]');
                    if (btn) { btn.click(); return JSON.stringify({ok: true, method: "button_click"}); }
                    var form = document.querySelector('form');
                    if (form) { form.submit(); return JSON.stringify({ok: true, method: "form_submit"}); }
                    return JSON.stringify({ok: false, error: "no submit button or form found"});
                })()
            ''') or '{"ok": false}'
        return result

    elif name == "find_and_click":
        tab = _get_tab()
        text = args["text"]
        role = args.get("role", "")
        nth = int(args.get("nth", 0))
        result = tab.js(f'''
            return (function() {{
                var role = {json.dumps(role)};
                var textQ = {json.dumps(text.lower())};
                var nth = {nth};
                var sel = role ? '[role=' + role + '],button,a,[role=button],[role=link]' : 'button,a,[role=button],[role=link],input[type=submit]';
                var els = Array.from(document.querySelectorAll(sel));
                var matches = els.filter(function(e) {{
                    return e.textContent.toLowerCase().indexOf(textQ) !== -1 ||
                           (e.getAttribute('aria-label')||'').toLowerCase().indexOf(textQ) !== -1;
                }});
                if (matches.length === 0) return JSON.stringify({{ok: false, error: "no match for: " + {json.dumps(text)}}});
                var target = matches[Math.min(nth, matches.length-1)];
                target.click();
                return JSON.stringify({{ok: true, text: target.textContent.trim().slice(0,60), nth: nth}});
            }})()
        ''') or '{"ok": false}'
        return result

    elif name == "login":
        import time as _time
        tab = _get_tab()
        url_arg = args["url"]
        email = args["email"]
        password = args["password"]
        tab.open(url_arg, wait_s=3.0)
        _time.sleep(1)
        # fill email
        tab.js(f'''
            (function() {{
                var el = document.querySelector('input[type=email],input[name=email],input[name=username],input[id*=email],input[id*=user]');
                if (!el) return;
                var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
                if (setter && setter.set) setter.set.call(el, {json.dumps(email)});
                else el.value = {json.dumps(email)};
                el.dispatchEvent(new Event('input', {{bubbles:true}}));
                el.dispatchEvent(new Event('change', {{bubbles:true}}));
            }})()
        ''')
        _time.sleep(0.5)
        # fill password
        tab.js(f'''
            (function() {{
                var el = document.querySelector('input[type=password]');
                if (!el) return;
                var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
                if (setter && setter.set) setter.set.call(el, {json.dumps(password)});
                else el.value = {json.dumps(password)};
                el.dispatchEvent(new Event('input', {{bubbles:true}}));
                el.dispatchEvent(new Event('change', {{bubbles:true}}));
            }})()
        ''')
        _time.sleep(0.3)
        # submit
        tab.js('''
            (function() {
                var btn = document.querySelector('button[type=submit],input[type=submit]');
                if (btn) btn.click();
                else { var f = document.querySelector('form'); if(f) f.submit(); }
            })()
        ''')
        _time.sleep(3)
        final_url = tab.js("return location.href") or ""
        title = tab.js("return document.title") or ""
        return json.dumps({"ok": True, "url": final_url, "title": title})

    elif name == "extract":
        tab = _get_tab()
        what = args.get("what", "links")
        if what == "links":
            result = tab.js('''
                return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,100).map(function(a){
                    return {text: a.textContent.trim().slice(0,80), href: a.href};
                }));
            ''') or '[]'
        else:  # tables
            result = tab.js('''
                return Array.from(document.querySelectorAll('table')).map(function(t){ return t.outerHTML; }).join('\\n');
            ''') or ''
        return result

    elif name == "extract_table":
        tab = _get_tab()
        selector = args.get("selector", "table")
        index = int(args.get("index", 0))
        result = tab.js(f'''
            return (function() {{
                var tables = document.querySelectorAll({json.dumps(selector)});
                var table = tables[{index}];
                if (!table) return JSON.stringify([]);
                var headers = Array.from(table.querySelectorAll('th')).map(function(th){{ return th.textContent.trim(); }});
                if (!headers.length) {{
                    var firstRow = table.querySelector('tr');
                    if (firstRow) headers = Array.from(firstRow.querySelectorAll('td')).map(function(td){{ return td.textContent.trim(); }});
                }}
                var rows = Array.from(table.querySelectorAll('tr')).slice(headers.length ? 1 : 0);
                var data = rows.map(function(row) {{
                    var cells = Array.from(row.querySelectorAll('td')).map(function(td){{ return td.textContent.trim(); }});
                    var obj = {{}};
                    cells.forEach(function(c, i){{ obj[headers[i] || i] = c; }});
                    return obj;
                }});
                return JSON.stringify(data);
            }})()
        ''') or '[]'
        return result

    elif name == "paginate":
        tab = _get_tab()
        selector = args.get("selector")
        if selector:
            result = tab.js(f'''
                return (function() {{
                    var el = document.querySelector({json.dumps(selector)});
                    if (!el) return JSON.stringify({{ok: false, error: "selector not found"}});
                    el.click();
                    return JSON.stringify({{ok: true, method: "custom_selector"}});
                }})()
            ''') or '{"ok": false}'
        else:
            result = tab.js('''
                return (function() {
                    var patterns = ['next','siguiente','→','›','>>','»','more','load more'];
                    var els = Array.from(document.querySelectorAll('a,button,[role=button]'));
                    for (var i=0; i<els.length; i++) {
                        var txt = els[i].textContent.toLowerCase().trim();
                        var aria = (els[i].getAttribute('aria-label')||'').toLowerCase();
                        for (var j=0; j<patterns.length; j++) {
                            if (txt === patterns[j] || aria === patterns[j]) {
                                els[i].click();
                                return JSON.stringify({ok: true, matched: patterns[j]});
                            }
                        }
                    }
                    // try rel=next link
                    var rel = document.querySelector('a[rel=next]');
                    if (rel) { rel.click(); return JSON.stringify({ok: true, method: "rel_next"}); }
                    return JSON.stringify({ok: false, error: "no next button found"});
                })()
            ''') or '{"ok": false}'
        return result

    elif name == "dismiss_overlay":
        tab = _get_tab()
        result = tab.js('''
            return (function() {
                var patterns = ['accept','acepto','aceptar','agree','i agree','ok','got it','close','cerrar','dismiss','reject','decline','deny','no thanks','continue'];
                var els = Array.from(document.querySelectorAll('button,a,[role=button]'));
                for (var i=0; i<els.length; i++) {
                    var txt = els[i].textContent.toLowerCase().trim();
                    var aria = (els[i].getAttribute('aria-label')||'').toLowerCase();
                    for (var j=0; j<patterns.length; j++) {
                        if (txt === patterns[j] || aria === patterns[j] || txt.indexOf(patterns[j]) !== -1) {
                            var s = window.getComputedStyle(els[i]);
                            if (s.display !== 'none' && s.visibility !== 'hidden') {
                                els[i].click();
                                return JSON.stringify({ok: true, clicked: els[i].textContent.trim().slice(0,40)});
                            }
                        }
                    }
                }
                return JSON.stringify({ok: false, error: "no overlay dismiss button found"});
            })()
        ''') or '{"ok": false}'
        return result

    elif name == "browse":
        import urllib.request as _req
        import urllib.error as _uerr
        url_arg = args["url"]
        headers = args.get("headers", {})
        try:
            request = _req.Request(url_arg, headers={"User-Agent": "Mozilla/5.0 (compatible; neo-browser/4)", **headers})
            with _req.urlopen(request, timeout=15) as resp:
                content_type = resp.headers.get("Content-Type", "")
                raw = resp.read(1024 * 512)  # 512KB max
                if "json" in content_type:
                    return raw.decode("utf-8", errors="replace")
                # strip HTML tags for text extraction
                text = raw.decode("utf-8", errors="replace")
                import re as _re
                text = _re.sub(r'<script[^>]*>.*?</script>', '', text, flags=_re.DOTALL | _re.IGNORECASE)
                text = _re.sub(r'<style[^>]*>.*?</style>', '', text, flags=_re.DOTALL | _re.IGNORECASE)
                text = _re.sub(r'<[^>]+>', ' ', text)
                text = _re.sub(r'\s+', ' ', text).strip()
                return json.dumps({"url": url_arg, "text": text[:8000], "content_type": content_type})
        except _uerr.URLError as e:
            return json.dumps({"ok": False, "error": str(e), "url": url_arg})

    elif name == "search":
        import urllib.parse as _parse
        import urllib.request as _req
        import re as _re
        query = args["query"]
        limit = int(args.get("limit", 10))
        encoded = _parse.quote_plus(query)
        url_ddg = f"https://html.duckduckgo.com/html/?q={encoded}"
        try:
            request = _req.Request(url_ddg, headers={"User-Agent": "Mozilla/5.0 (compatible; neo-browser/4)"})
            with _req.urlopen(request, timeout=15) as resp:
                html = resp.read(512 * 1024).decode("utf-8", errors="replace")
            results = []
            # split on result__body (class may have extra tokens like "links_main links_deep result__body")
            blocks = html.split('result__body')
            for block in blocks[1:limit+1]:
                title_m = _re.search(r'<a[^>]*class="result__a"[^>]*>(.+?)</a>', block, _re.DOTALL)
                # DDG wraps URLs in redirect: uddg=<encoded_url>
                url_m   = _re.search(r'uddg=([^&"]+)', block)
                snip_m  = _re.search(r'class="result__snippet"[^>]*>(.+?)</a>', block, _re.DOTALL)
                title   = _re.sub(r'<[^>]+>', '', title_m.group(1)).strip() if title_m else ""
                href    = _parse.unquote(url_m.group(1)) if url_m else ""
                snippet = _re.sub(r'<[^>]+>', '', snip_m.group(1)).strip() if snip_m else ""
                if title:
                    results.append({"title": title, "url": href, "snippet": snippet})
            return json.dumps({"query": query, "results": results[:limit]})
        except Exception as e:
            return json.dumps({"ok": False, "error": str(e)})

    elif name == "debug":
        tab = _get_tab()
        action = args.get("action", "flush")
        if action == "start":
            tab.js('''
                if (!window.__neo_debug_logs) window.__neo_debug_logs = [];
                window.__neo_debug_orig = {log: console.log, warn: console.warn, error: console.error};
                ['log','warn','error'].forEach(function(l) {
                    console[l] = function() {
                        var msg = Array.from(arguments).map(function(a){ try{return JSON.stringify(a);}catch(e){return String(a);} }).join(' ');
                        window.__neo_debug_logs.push({level: l, msg: msg, t: Date.now()});
                        window.__neo_debug_orig[l].apply(console, arguments);
                    };
                });
            ''')
            return json.dumps({"ok": True, "action": "interceptor_installed"})
        elif action == "flush":
            result = tab.js('''
                var logs = window.__neo_debug_logs || [];
                window.__neo_debug_logs = [];
                return JSON.stringify(logs);
            ''') or '[]'
            return result
        else:  # stop
            tab.js('''
                if (window.__neo_debug_orig) {
                    console.log = window.__neo_debug_orig.log;
                    console.warn = window.__neo_debug_orig.warn;
                    console.error = window.__neo_debug_orig.error;
                    delete window.__neo_debug_orig;
                }
                window.__neo_debug_logs = [];
            ''')
            return json.dumps({"ok": True, "action": "interceptor_removed"})

    raise ValueError(f"Unknown tool: {name}")


# ---------------------------------------------------------------------------
# MCP Protocol (JSON-RPC 2.0 over stdin/stdout)
# ---------------------------------------------------------------------------


def _respond(req_id: Any, result: Any) -> None:
    line = json.dumps({"jsonrpc": "2.0", "id": req_id, "result": result})
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def _respond_error(req_id: Any, code: int, message: str) -> None:
    line = json.dumps({"jsonrpc": "2.0", "id": req_id, "error": {"code": code, "message": message}})
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def _get_mcp_tools() -> list[dict]:
    result = []
    for name, t in TOOLS.items():
        properties: dict = {}
        required: list = []
        for param, spec in t["schema"].items():
            prop = {"type": spec.get("type", "string"), "description": spec["description"]}
            if "enum" in spec:
                prop["enum"] = spec["enum"]
            if spec.get("required"):
                required.append(param)
            properties[param] = prop
        result.append({
            "name": name,
            "description": t["description"],
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        })
    return result


def _handle(req: dict) -> None:
    method = req.get("method", "")
    params = req.get("params", {})
    req_id = req.get("id")

    if method == "initialize":
        _respond(req_id, {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": VERSION},
        })

    elif method == "tools/list":
        _respond(req_id, {"tools": _get_mcp_tools()})

    elif method == "tools/call":
        tool_name = params.get("name", "")
        tool_args = params.get("arguments", {})
        if tool_name not in TOOLS:
            _respond_error(req_id, -32601, f"Unknown tool: {tool_name}")
            return
        try:
            result = dispatch_tool(tool_name, tool_args)
            if result is None:
                result = ""
            text = result if isinstance(result, str) else json.dumps(result, ensure_ascii=False)
            if len(text) > 500_000:
                text = text[:500_000] + f"\n... (truncated from {len(text)} chars)"
            _respond(req_id, {"content": [{"type": "text", "text": text}]})
        except Exception as exc:
            _respond(req_id, {
                "content": [{"type": "text", "text": f"Error: {exc}\n{traceback.format_exc()}"}],
                "isError": True,
            })

    elif method == "notifications/initialized":
        pass  # client notification, no response needed

    elif req_id is not None:
        _respond_error(req_id, -32601, f"Unknown method: {method}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _doctor() -> None:
    print(f"NeoBrowser V4 — {VERSION}")
    print()

    # Python
    import platform
    print(f"Python: {platform.python_version()} {'OK' if sys.version_info >= (3, 10) else 'NEED 3.10+'}")

    # websockets
    try:
        import websockets
        print(f"websockets: {websockets.__version__} OK")
    except ImportError:
        print("websockets: MISSING — pip install websockets")

    # anthropic
    try:
        import anthropic
        print(f"anthropic: {anthropic.__version__} OK")
    except ImportError:
        print("anthropic: MISSING — pip install anthropic (needed for LLM fallback in PageAnalyzer)")

    # Chrome
    from tools.v4.chrome_process import CHROME_BIN
    chrome_ok = os.path.exists(CHROME_BIN)
    print(f"Chrome: {'OK' if chrome_ok else 'NOT FOUND'} ({CHROME_BIN})")

    # Browser
    print()
    print("V4 modules: tools/v4/browser.py, session.py, tab_pool.py, page_analyzer.py,")
    print("            chrome_tab.py, chrome_process.py, playbook.py, lifecycle.py")

    print()
    if chrome_ok:
        print("Status: READY")
    else:
        print("Status: Chrome not found — set NEOBROWSER_CHROME_BIN env var")


def main() -> None:
    if len(sys.argv) > 1:
        arg = sys.argv[1]
        if arg in ("--version", "-v"):
            print(f"{VERSION}")
            return
        if arg in ("--help", "-h"):
            print(__doc__)
            return
        if arg == "doctor":
            _doctor()
            return

    # MCP server mode — read JSON-RPC from stdin
    _log_level = os.environ.get("NEO_LOG_LEVEL", "DEBUG").upper()
    logging.basicConfig(
        level=getattr(logging, _log_level, logging.DEBUG),
        stream=sys.stderr,
        format="[neo-v4] %(levelname)s %(name)s: %(message)s",
    )
    # Also write to file for debugging (readable by developer tools)
    _fh = logging.FileHandler("/tmp/neo_v4_debug.log", mode="a")
    _fh.setLevel(logging.DEBUG)
    _fh.setFormatter(logging.Formatter("[neo-v4] %(asctime)s %(levelname)s %(name)s: %(message)s"))
    logging.getLogger().addHandler(_fh)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            _handle(req)
        except json.JSONDecodeError as exc:
            _respond_error(None, -32700, f"Parse error: {exc}")
        except Exception as exc:
            _respond_error(None, -32603, f"Internal error: {exc}")


if __name__ == "__main__":
    main()
