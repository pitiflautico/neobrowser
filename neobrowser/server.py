"""
NeoBrowser — MCP server (stdin/stdout JSON-RPC 2.0).

Exposes a real, stealth-hardened Chrome (driven over the Chrome DevTools
Protocol) as MCP tools so AI models can navigate the web autonomously —
optionally reusing the user's real logged-in Chrome sessions.

Tool groups:
  navigation      navigate, scroll, wait, page_info
  observation     read, screenshot, extract, extract_table, console_logs,
                  network_log, metrics, analyze, find
  interaction     click, type, fill, form_fill, submit, find_and_click, login
  session         save_cookies, restore_cookies, save_session, session_info
  playbooks       record_task, stop_recording, replay
  web/search      browse, search, search_images, search_videos

Usage:
  neobrowser              # start the MCP server (reads JSON-RPC from stdin)
  neobrowser --version    # print version
  neobrowser doctor       # check dependencies and Chrome

MCP client config:
  { "neobrowser": { "command": "neobrowser" } }
"""
from __future__ import annotations

import json
import sys
import os
import traceback
import logging
from typing import Any

# Ensure repo root (parent of the neobrowser package) on path when run directly
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

VERSION = "1.0.0"
SERVER_NAME = "neobrowser"
log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Global Browser instance — one per server process
# ---------------------------------------------------------------------------

_browser = None  # type: ignore[assignment]
_current_tab = None  # type: ignore[assignment]
_cookies_injected: bool = False  # guard: inject real-Chrome cookies once per Browser
_cookie_injection_attempts: int = 0  # bounded retry budget, reset when the Browser is replaced


def _drop_browser() -> None:
    """
    Close and forget the current Browser, resetting its per-Browser state.

    Recovery paths (dead Chrome / stale WebSocket) replace the Browser; without
    closing the old one first its spawned Chrome process is orphaned on every
    cycle. Also resets the cookie-injection guard so real-Chrome cookies are
    re-injected into the fresh Browser.
    """
    global _browser, _current_tab, _cookies_injected, _cookie_injection_attempts
    old = _browser
    _browser = None
    _current_tab = None
    _cookies_injected = False
    _cookie_injection_attempts = 0
    if old is not None:
        try:
            old.close()
        except Exception:
            pass


def _resolve_attach_port() -> int | None:
    """
    Resolve which Chrome port to attach to, in priority order:
    1. NEOBROWSER_ATTACH_PORT env var (explicit override)
    2. The port handoff file under NEOBROWSER_HOME (dynamic, read at call time)
    Returns None if no reachable Chrome is found.
    """
    import urllib.request as _ur

    def _reachable(port: int) -> bool:
        # Verify it's genuinely a Chrome DevTools endpoint, not just any HTTP 200
        # process that happens to hold the port.
        try:
            with _ur.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.0) as resp:
                info = json.loads(resp.read().decode())
            browser = info.get("Browser", "")
            return (
                browser.startswith("Chrome/")
                or browser.startswith("Chromium/")
                or browser.startswith("HeadlessChrome/")
            )
        except Exception:
            return False

    # 1. Explicit env var
    env_port = os.environ.get("NEOBROWSER_ATTACH_PORT")
    if env_port:
        p = int(env_port)
        if _reachable(p):
            return p

    # 2. Port handoff file (re-read every time — an attached Chrome may restart)
    from neobrowser.paths import PORT_FILE
    port_file = str(PORT_FILE)
    if os.path.exists(port_file):
        try:
            p = int(open(port_file).read().strip())
            if _reachable(p):
                return p
        except Exception:
            pass

    return None


def _get_browser():
    global _browser, _current_tab
    if _browser is not None:
        # Health check: verify the cached browser's Chrome port is still alive
        try:
            session = getattr(_browser, "_session", None)
            port = getattr(session, "_port", None)
            if port:
                import urllib.request as _ur
                _ur.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.0)
        except Exception:
            # Chrome died or changed port — force full re-resolution
            log.warning("Cached browser port unreachable, re-resolving Chrome...")
            _drop_browser()

    if _browser is None:
        from neobrowser.browser import Browser
        pool_size = int(os.environ.get("NEOBROWSER_POOL_SIZE", "3"))
        attach_port = _resolve_attach_port()
        if attach_port:
            _browser = Browser.connect(attach_port, pool_size=pool_size)
            log.info("Browser attached (port=%s, pool=%d)", attach_port, pool_size)
        else:
            profile = os.environ.get("NEOBROWSER_PROFILE", "default")
            visible = os.environ.get("NEOBROWSER_VISIBLE", "") == "1"
            _browser = Browser(profile=profile, pool_size=pool_size, visible=visible)
            log.info("Browser started (profile=%s, pool=%d, visible=%s)", profile, pool_size, visible)
    return _browser


def _inject_real_chrome_cookies_once(tab) -> None:
    """
    Inject decrypted real-Chrome cookies into tab, once per Browser.

    Only marks the guard done on SUCCESS. A failure (e.g. a transient CDP
    timeout) is retried on the next tab acquisition, up to a small budget,
    instead of permanently disabling real-session auth for the whole run.
    """
    global _cookies_injected, _cookie_injection_attempts
    if _cookies_injected or _cookie_injection_attempts >= 3:
        return
    try:
        from neobrowser.cookie_sync import inject_from_real_chrome
        b = _get_browser()
        profile_name = getattr(getattr(b, "_session", None), "profile_name", "default")
        injected = inject_from_real_chrome(tab, profile_name)
        _cookies_injected = True
        log.info("_inject_real_chrome_cookies_once: injected %d cookies (profile=%s)", injected, profile_name)
    except Exception as exc:
        _cookie_injection_attempts += 1
        log.warning(
            "_inject_real_chrome_cookies_once failed (attempt %d/3, non-fatal): %s",
            _cookie_injection_attempts, exc,
        )


def _get_tab(url: str | None = None, wait_s: float = 3.0):
    """Get current tab, navigating if url provided. Auto-recovers stale WebSocket or dead Chrome."""
    global _current_tab, _browser

    def _fresh_browser():
        """Close the dead Browser, re-resolve Chrome, and create a fresh one."""
        _drop_browser()
        return _get_browser()

    b = _get_browser()
    if _current_tab is None:
        _current_tab = b.open(url or "about:blank", wait_s=wait_s if url else 0)
        _inject_real_chrome_cookies_once(_current_tab)
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
        "description": "Open URL in Chrome (tab pool reuse, AX cache, thread-safe). Required for SPAs, JS-heavy sites, and login-required pages.",
        "schema": {
            "url":    {"type": "string",  "description": "HTTP/HTTPS URL to open", "required": True},
            "wait_s": {"type": "number",  "description": "Seconds to wait for page render (default 3.0)"},
        },
    },
    "screenshot": {
        "description": "Capture current page viewport as base64 PNG. also supports JPEG.",
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
        "description": "Type text into the currently focused element. Default: instant insert (React/Vue-safe). Set human=true for per-key events with human cadence (slower; for sites with keystroke-timing analysis).",
        "schema": {
            "text": {"type": "string", "description": "Text to type", "required": True},
            "human": {"type": "boolean", "description": "Type key-by-key with human-like timing (default false)"},
        },
    },
    "console_logs": {
        "description": "Get captured browser console log entries (log/warning/error/exception).",
        "schema": {
            "level": {"type": "string", "description": "Filter by level: log, info, warning, error (default: all)"},
            "limit": {"type": "integer","description": "Max entries to return (default 50)"},
        },
    },
    "network_log": {
        "description": "Get captured network requests with status, duration, size.",
        "schema": {
            "url_pattern": {"type": "string", "description": "Filter by URL substring (default: all)"},
            "limit":       {"type": "integer","description": "Max entries (default 50)"},
        },
    },
    "metrics": {
        "description": "Get Chrome performance metrics: JSHeapUsedSize, Nodes, Documents, etc.",
        "schema": {
            "key": {"type": "string", "description": "Return only this metric (default: all)"},
        },
    },
    "save_cookies": {
        "description": "Save current session cookies to ~/.neobrowser/cookies/{profile}.json (0600 perms).",
        "schema": {},
    },
    "restore_cookies": {
        "description": "Inject saved cookies from disk into current tab. Returns count restored.",
        "schema": {},
    },
    "save_session": {
        "description": "Full session save: cookies + localStorage → ~/.neobrowser/sessions/. Persists authenticated state so future restarts are pre-authenticated.",
        "schema": {},
    },
    "session_info": {
        "description": "Show session persistence state: last sync time, cookie count, domains, file paths.",
        "schema": {},
    },
    "record_task": {
        "description": "Start recording interaction steps as a playbook for future replay.",
        "schema": {
            "domain":    {"type": "string", "description": "Domain key, e.g. 'linkedin.com'", "required": True},
            "task_name": {"type": "string", "description": "Task identifier, e.g. 'send_message'", "required": True},
        },
    },
    "stop_recording": {
        "description": "Stop recording and save playbook to disk. Returns step count.",
        "schema": {},
    },
    "replay": {
        "description": "Replay a saved playbook. Returns {ok, first_failed_step}.",
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
        "description": "Detect and dismiss cookie banners, GDPR modals, newsletter popups and other overlays that block interaction. Targets real overlays (fixed/sticky, high z-index) and clicks Accept/Close inside them. Try this if clicks aren't working.",
        "schema": {
            "force": {"type": "boolean", "description": "Try harder: also send Escape and click the backdrop (default false)"},
        },
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
    "search_images": {
        "description": (
            "Search Google Images. Returns image results with direct download URL, source page, "
            "title and description. Each result includes a ready-to-run curl download command."
        ),
        "schema": {
            "query": {"type": "string", "description": "Image search query", "required": True},
            "count": {"type": "integer", "description": "Number of results (default 10, max 30)"},
        },
    },
    "search_videos": {
        "description": (
            "Search Google Videos. Returns video results with title, URL, channel, duration, "
            "description, platform (youtube/instagram/tiktok/…) and a yt-dlp download command."
        ),
        "schema": {
            "query": {"type": "string", "description": "Video search query", "required": True},
            "count": {"type": "integer", "description": "Number of results (default 10, max 30)"},
        },
    },
    "search_twitter_videos": {
        "description": (
            "Search Twitter/X for video tweets. Returns tweets containing video with author, "
            "text, metrics (replies/reposts/likes/views), tweet URL, and a yt-dlp download command. "
            "Use for finding viral clips, news footage, tutorials, or any video content shared on Twitter."
        ),
        "schema": {
            "query": {"type": "string", "description": "Twitter video search query", "required": True},
            "count": {"type": "integer", "description": "Number of results (default 10, max 30)"},
        },
    },
    "debug": {
        "description": "Capture browser console errors/logs. Installs interceptor and flushes buffered messages.",
        "schema": {
            "action": {"type": "string", "description": "Action: start (install interceptor), flush (get buffered logs), stop", "enum": ["start", "flush", "stop"]},
        },
    },
}


# ---------------------------------------------------------------------------
# Tool dispatch
# ---------------------------------------------------------------------------


_AUTH_WALL_JS = r"""
return (function(){
    const path = location.pathname.toLowerCase();
    const hasPassword = !!document.querySelector('input[type=password]');
    const loginPath = /(^|\/)(login|signin|sign-in|sign_in|auth|sso|account\/login)(\/|$)/.test(path);
    const body = (document.body ? document.body.innerText : '').toLowerCase().slice(0, 3000);
    const signals = ['verify you are human','are you a robot','are you human','i am not a robot',
        'checking your browser','cloudflare','captcha','hcaptcha','recaptcha',
        'access denied','unusual traffic','confirm you are not a robot'];
    let challenge = null;
    for (const s of signals) { if (body.includes(s)) { challenge = s; break; } }
    return JSON.stringify({hasPassword: hasPassword, loginPath: loginPath, challenge: challenge});
})()
"""


def _detect_auth_wall(tab):
    """
    Heuristically detect a login wall or bot challenge on the current page, so
    the model knows it is blocked instead of silently acting on a logged-out
    page. Returns a small dict, or None if the page looks accessible. Never raises.
    """
    try:
        raw = tab.js(_AUTH_WALL_JS)
        d = json.loads(raw) if raw else {}
    except Exception:
        return None
    if d.get("challenge"):
        return {"kind": "bot_challenge", "signal": d["challenge"],
                "hint": "A bot/CAPTCHA challenge is showing — real-session mode (NEOBROWSER_REAL_PROFILE) or a visible login may be needed."}
    if d.get("hasPassword") and d.get("loginPath"):
        return {"kind": "login_wall",
                "hint": "This looks like a login page — set NEOBROWSER_REAL_PROFILE to reuse a logged-in session, or use the login tool."}
    return None


def _record_if_recording(action: str, params: dict, fallback: dict | None = None) -> None:
    """If a playbook recording is active, append this action as a step so
    record_task -> ...actions... -> stop_recording actually captures them."""
    if _browser is None or not getattr(_browser, "_recording_domain", None):
        return
    try:
        from neobrowser.playbook import Step
        _browser.record_step(Step(action, params, fallback))
    except Exception:
        pass


def _search_google(tab, query: str, limit: int) -> list:
    """
    Text search via Google, using the real stealth browser. Returns [] if Google
    shows its /sorry/ bot wall (clean profiles) — the caller then falls back to
    DuckDuckGo. With NEOBROWSER_REAL_PROFILE set, the user's logged-in Google
    session sails past that wall and this returns real results.
    """
    import urllib.parse as _parse
    from neobrowser.google_search import _dismiss_consent
    tab.navigate(f"https://www.google.com/search?q={_parse.quote_plus(query)}&hl=en&num=20", wait_s=3.0)
    try:
        _dismiss_consent(tab)
    except Exception:
        pass
    url = tab.js("return location.href") or ""
    if "/sorry/" in url or "consent.google" in url:
        return []
    raw = tab.js(r"""return JSON.stringify((function(limit){
        const out = [], seen = new Set();
        document.querySelectorAll('a h3').forEach(function(h3){
            if (out.length >= limit) return;
            const a = h3.closest('a[href]'); if (!a) return;
            let href = a.href || '';
            if (!href || href.indexOf('https://www.google.') === 0 || seen.has(href)) return;
            seen.add(href);
            let snip = '';
            const c = a.closest('div.g, div.MjjYud, div[data-hveid]');
            if (c) { const s = c.querySelector('.VwiC3b, div[data-sncf], span'); if (s) snip = s.textContent.slice(0,220); }
            out.push({title: h3.textContent.trim(), url: href, snippet: snip.trim()});
        });
        return out;
    })(%d))""" % limit)
    try:
        return json.loads(raw) if raw else []
    except Exception:
        return []


def _search_duckduckgo(tab, query: str, limit: int) -> list:
    """Text search via DuckDuckGo's no-JS endpoint through the real browser
    (its genuine Chrome headers aren't blocked, unlike a raw HTTP fetch)."""
    import urllib.parse as _parse
    tab.navigate(f"https://html.duckduckgo.com/html/?q={_parse.quote_plus(query)}", wait_s=3.0)
    raw = tab.js(r"""return JSON.stringify((function(limit){
        const out = [], seen = new Set();
        document.querySelectorAll('.result__body, .result').forEach(function(r){
            if (out.length >= limit) return;
            if ((r.className || '').indexOf('result--ad') !== -1) return;   // skip sponsored
            const a = r.querySelector('.result__a'); if (!a) return;
            let href = a.href || '';
            if (href.indexOf('/y.js') !== -1 || href.indexOf('ad_domain') !== -1) return;  // ad redirect
            try { const u = new URL(href); if (u.searchParams.get('uddg')) href = u.searchParams.get('uddg'); } catch(e){}
            if (!href || seen.has(href)) return;
            seen.add(href);
            const sn = r.querySelector('.result__snippet');
            out.push({title: a.textContent.trim(), url: href, snippet: sn ? sn.textContent.trim() : ''});
        });
        return out;
    })(%d))""" % limit)
    try:
        return json.loads(raw) if raw else []
    except Exception:
        return []


def dispatch_tool(name: str, args: dict) -> Any:
    if name in _PLUGIN_HANDLERS:
        return _PLUGIN_HANDLERS[name](args)

    b = _get_browser()

    if name == "navigate":
        url = args["url"]
        wait_s = float(args.get("wait_s", 3.0))
        tab = _get_tab(url, wait_s=wait_s)
        _record_if_recording("navigate", {"url": url})
        msg = f"Navigated to {tab.current_url()}"
        wall = _detect_auth_wall(tab)
        if wall:
            msg += f"\n⚠️ {wall['kind']}: {wall['hint']}"
        else:
            # Auth-wall covers login/captcha; classify_page_state adds the other
            # states (rate-limited, server error) so the model isn't misled.
            from neobrowser.perception import classify_page_state
            snippet = tab.js("return (document.body ? document.body.innerText.slice(0,3000) : '')") or ""
            state = classify_page_state(snippet)
            if state in ("rate_limited", "error"):
                msg += f"\n⚠️ page looks {state.replace('_', '-')}"
        return msg

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
        from neobrowser.page_analyzer import FormFinder
        finder = FormFinder(tab)
        result = finder.find(intent)
        if result is None:
            return json.dumps({"found": False, "backend_node_id": None})
        return json.dumps({"found": True, **result.to_dict()})

    elif name == "click":
        import time as _t
        from neobrowser.perception import CLICK_SNAPSHOT_JS, click_outcome
        tab = _get_tab()
        node_id = args.get("backend_node_id")
        selector = args.get("selector")
        before = json.loads(tab.js(CLICK_SNAPSHOT_JS) or "{}")
        clicked = None
        if node_id is not None:
            # Prefer real mouse events (isTrusted) for stealth; fall back to JS
            # click if the element has no layout box (off-screen / display:none).
            if tab.click_node_real(int(node_id)):
                _record_if_recording("click_node", {"backend_node_id": int(node_id)})
                clicked = f"node {node_id}"
            else:
                result = tab.send("DOM.resolveNode", {"backendNodeId": int(node_id)})
                obj_id = result.get("object", {}).get("objectId")
                if obj_id:
                    tab.send("Runtime.callFunctionOn", {
                        "objectId": obj_id,
                        "functionDeclaration": "function(){this.click()}",
                        "returnByValue": True,
                    })
                    _record_if_recording("click_node", {"backend_node_id": int(node_id)})
                    clicked = f"node {node_id}"
                else:
                    return json.dumps({"clicked": False, "error": f"node {node_id} not found in DOM"})
        elif selector:
            if tab.click(selector):
                clicked = selector
            else:
                return json.dumps({"clicked": False, "error": f"selector not found: {selector}"})
        else:
            return json.dumps({"clicked": False, "error": "provide backend_node_id or selector"})
        # Report what the click actually did so the model isn't blind post-action.
        _t.sleep(0.4)
        after = json.loads(tab.js(CLICK_SNAPSHOT_JS) or "{}")
        outcome, extra = click_outcome(before, after)
        return json.dumps({"clicked": clicked, "outcome": outcome, **extra})

    elif name == "type":
        tab = _get_tab()
        text = args["text"]
        if args.get("human"):
            # Per-key events with human cadence (isTrusted) — for sites with
            # keystroke-timing analysis. Slower; opt-in.
            tab.type_humanlike(text)
        else:
            # Instant insert — fast and React/Vue-safe (default).
            tab.send("Input.insertText", {"text": text})
        _record_if_recording("type", {"text": text})
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
        # enable_network() may not be on _AcquiredTab proxy — call via send() directly
        try:
            if not getattr(tab, "_network_enabled", False):
                tab.enable_network()
        except AttributeError:
            try:
                tab.send("Network.enable", {})
            except Exception:
                pass
        try:
            reqs = b.network_log(tab)
        except AttributeError:
            # fallback: access inner tab if proxy wraps it
            inner = getattr(tab, "_tab", tab)
            reqs = getattr(inner, "_network_requests", [])
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
        import time as _time
        tab = _get_tab()
        selector = args.get("selector")
        max_wait_s = float(args.get("wait_s", 5.0))
        url_before = tab.js("return location.href") or ""

        if selector:
            tab.js(f'''
                var el = document.querySelector({json.dumps(selector)});
                if (el) el.click();
            ''')
            method = selector
        else:
            method = tab.js('''
                return (function() {
                    var btn = document.querySelector('button[type=submit],input[type=submit]');
                    if (btn) { btn.click(); return "button_click"; }
                    var btn2 = document.querySelector('[aria-label*="submit" i],[aria-label*="send" i]');
                    if (btn2) { btn2.click(); return "aria_button"; }
                    var form = document.querySelector('form');
                    if (form) { form.submit(); return "form_submit"; }
                    return null;
                })()
            ''') or ""
            if not method:
                return json.dumps({"ok": False, "error": "no submit button or form found"})

        # Wait for navigation or readyState=complete (replaces hardcoded sleep)
        t0 = _time.time()
        url_after = url_before
        for _ in range(int(max_wait_s * 10)):
            _time.sleep(0.1)
            try:
                ready = tab.js("return document.readyState")
                url_now = tab.js("return location.href") or url_before
                if url_now != url_before or ready == "complete":
                    url_after = url_now
                    break
            except Exception:
                break
        waited_ms = round((_time.time() - t0) * 1000)
        return json.dumps({"ok": True, "method": method, "url": url_after, "waited_ms": waited_ms})

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
        from urllib.parse import urlparse as _urlparse
        url_arg = args["url"]
        email = args["email"]
        password = args["password"]
        # Never send credentials over plaintext, and never to a non-web scheme.
        if _urlparse(url_arg).scheme != "https":
            return json.dumps({"ok": False, "error": "login requires an https:// URL"})
        # _get_tab(url) navigates the pooled tab (the old tab.open() call was a
        # classmethod and crashed on every invocation).
        tab = _get_tab(url_arg, wait_s=3.0)
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
        # Honest success signal: a lingering password field usually means the
        # login did not complete (bad credentials, an extra step, or a challenge)
        # — report that instead of a blind ok:True.
        still_login = bool(tab.js("return !!document.querySelector('input[type=password]')"))
        return json.dumps({
            "ok": not still_login,
            "url": final_url,
            "title": title,
            "still_has_password_field": still_login,
        })

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
        from neobrowser.perception import DISMISS_OVERLAY_JS
        tab = _get_tab()
        force = "true" if args.get("force") else "false"
        # Target only real overlays (fixed/sticky, high z-index, visible) and
        # click accept/close INSIDE them — safer than clicking any matching
        # button anywhere on the page. force=true also tries Escape + backdrop.
        return tab.js(DISMISS_OVERLAY_JS.replace("FORCE", force)) or '{"dismissed": false}'

    elif name == "browse":
        import urllib.request as _req
        from urllib.parse import urlparse as _urlparse
        url_arg = args["url"]
        headers = args.get("headers", {})
        # Only fetch over http(s) — block file://, ftp://, data:, etc.
        # (arbitrary local-file read / SSRF via a caller- or page-supplied URL).
        if _urlparse(url_arg).scheme.lower() not in ("http", "https"):
            return json.dumps({"ok": False, "error": "browse only supports http(s) URLs", "url": url_arg})
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
        except (OSError, UnicodeDecodeError) as e:
            # OSError covers URLError, socket.timeout, and TimeoutError.
            return json.dumps({"ok": False, "error": str(e), "url": url_arg})

    elif name == "search":
        query = args["query"]
        limit = int(args.get("limit", 10))
        tab = _get_tab()
        # Drive the real stealth browser (a raw HTTP fetch gets bot-blocked).
        # Try Google first — it works when NEOBROWSER_REAL_PROFILE is set (the
        # user's logged-in session avoids Google's /sorry/ wall). Fall back to
        # DuckDuckGo, which serves results to any genuine browser.
        engine = "google"
        results = _search_google(tab, query, limit)
        if not results:
            engine = "duckduckgo"
            results = _search_duckduckgo(tab, query, limit)
        return json.dumps({"query": query, "engine": engine, "results": results[:limit]})

    elif name == "search_images":
        from neobrowser import google_search as _gs
        tab = _get_tab()
        query = args["query"]
        count = int(args.get("count", 10))
        results = _gs.search_images(tab, query, count)
        return json.dumps({"query": query, "count": len(results), "results": results}, ensure_ascii=False)

    elif name == "search_videos":
        from neobrowser import google_search as _gs
        tab = _get_tab()
        query = args["query"]
        count = int(args.get("count", 10))
        results = _gs.search_videos(tab, query, count)
        return json.dumps({"query": query, "count": len(results), "results": results}, ensure_ascii=False)

    elif name == "search_twitter_videos":
        from neobrowser import twitter_search as _ts
        query = args["query"]
        count = int(args.get("count", 10))
        # twitter_search manages its own visible Chrome (no headless tab)
        results = _ts.search_twitter_videos(query, count)
        return json.dumps({"query": query, "count": len(results), "results": results}, ensure_ascii=False)

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
# Plugin system — optional private extensions (e.g. gpt/grok chat)
#
# Absent in the public repo (no _private_chat.py present) this is a no-op:
# TOOLS simply lacks the plugin-provided entries and dispatch_tool never
# matches them. When neobrowser/_private_chat.py is present, its register()
# is called and gpt/grok become available like any other tool.
# ---------------------------------------------------------------------------

_PLUGIN_HANDLERS: dict = {}


def _register_plugin_tool(name, spec, handler) -> None:
    TOOLS[name] = spec
    _PLUGIN_HANDLERS[name] = handler


def _load_plugins() -> None:
    try:
        from neobrowser import _private_chat
        _private_chat.register(_register_plugin_tool, _get_browser)
    except ImportError:
        pass
    except Exception as exc:
        log.warning("plugin load failed: %s", exc)


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
            # Screenshots are binary base64 — return them as MCP image content so
            # they are never corrupted by the text length cap below (a mid-base64
            # slice both breaks the image and can emit invalid JSON).
            if tool_name == "screenshot":
                try:
                    shot = json.loads(result) if isinstance(result, str) else result
                    mime = "image/jpeg" if shot.get("format") == "jpeg" else "image/png"
                    _respond(req_id, {"content": [{"type": "image", "data": shot["data"], "mimeType": mime}]})
                    return
                except Exception:
                    pass  # fall through to text handling on any unexpected shape
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


# Load optional private plugins now — everything they may need
# (_get_browser, TOOLS, dispatch_tool) is already defined above.
_load_plugins()


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _doctor() -> None:
    print(f"NeoBrowser — {VERSION}")
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
    from neobrowser.chrome_process import CHROME_BIN
    chrome_ok = os.path.exists(CHROME_BIN)
    print(f"Chrome: {'OK' if chrome_ok else 'NOT FOUND'} ({CHROME_BIN})")

    # Modules
    print()
    print("Modules: browser, session, tab_pool, page_analyzer, chrome_tab,")
    print("         chrome_process, cookie_sync, playbook, lifecycle")

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
    _log_level = os.environ.get(
        "NEOBROWSER_LOG_LEVEL", os.environ.get("NEO_LOG_LEVEL", "INFO")
    ).upper()
    logging.basicConfig(
        level=getattr(logging, _log_level, logging.INFO),
        stream=sys.stderr,
        format="[neobrowser] %(levelname)s %(name)s: %(message)s",
    )
    # Optional rotating debug log — opt in via NEOBROWSER_DEBUG_LOG (=1 for a
    # tempdir file, or an absolute path). Off by default; bounded when on.
    _debug_log = os.environ.get("NEOBROWSER_DEBUG_LOG")
    if _debug_log:
        import tempfile
        from logging.handlers import RotatingFileHandler
        log_path = (
            _debug_log if os.path.isabs(_debug_log)
            else os.path.join(tempfile.gettempdir(), "neobrowser_debug.log")
        )
        _fh = RotatingFileHandler(log_path, maxBytes=10 * 1024 * 1024, backupCount=3)
        _fh.setLevel(getattr(logging, _log_level, logging.INFO))
        _fh.setFormatter(logging.Formatter("[neobrowser] %(asctime)s %(levelname)s %(name)s: %(message)s"))
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
