"""
tools/v4/chrome_tab.py

Tier 1: Isolated CDP connection to a single Chrome tab.

Fixes V3 bugs:
- V3 _active race condition: V3's GhostChrome.ws returns self._tabs[self._active];
  any thread can change _active between the read and the send. V4: each ChromeTab
  owns its own WebSocket — no shared _active pointer.
- V3 _recv_lock shared across all tabs: one RLock for ALL tabs blocks Tab B while
  Tab A is waiting for a response. V4: each ChromeTab has its own _lock.
- V3 querySelector returns FIRST element: for "latest message" semantics we need
  the LAST element. V4 wait_last() uses querySelectorAll + els[els.length-1].
- V3 no health check: if WS dies mid-session there is no detection. V4 exposes ping().

F03 additions:
- Background reader thread owns all ws.recv() calls; routes CDP responses to
  per-request queues and events to the page-event handler.
- current_url(), page_title(), navigation_history(), is_at() from Page events.
"""
from __future__ import annotations

import json
import queue
import threading
import time
import urllib.request
from typing import Any


class ChromeTab:
    """
    Isolated CDP connection to a single Chrome tab.

    Each instance has its own WebSocket, lock, and ID counter.
    No shared state with other tabs — fixes V3's _active race condition.

    F03: A background reader thread owns all ws.recv() calls, routing CDP
    command responses to per-request queues and CDP events to the page
    event handler. This allows Page.frameNavigated events to arrive and be
    processed even while the caller is sleeping between send() calls.
    """

    _SEND_TIMEOUT = 30.0  # seconds to wait for a CDP response

    def __init__(self, ws: Any, tab_id: str, port: int) -> None:
        """
        Parameters
        ----------
        ws:     websockets.sync.client connection (or any object with
                send(str) / recv() methods — makes mocking straightforward)
        tab_id: Chrome target ID (from /json/new response)
        port:   Chrome remote-debugging port (needed to close the tab)
        """
        self._ws = ws
        self._tab_id = tab_id
        self._port = port
        # Set to True by attach() — close() will only disconnect the WS,
        # not destroy the Chrome tab (which the caller owns).
        self._is_attached: bool = False
        self._lock = threading.RLock()   # serialises send() — per-tab, not shared
        self._id_counter = 0

        # F03: per-request response queues — keyed by message id
        # send() registers a queue before sending, reader thread delivers to it.
        self._response_queues: dict[int, queue.Queue] = {}
        self._rq_lock = threading.Lock()  # guards _response_queues dict

        # F03: URL state & navigation history
        self._current_url: str | None = None
        self._page_title: str | None = None
        self._history: list[str] = []
        self._page_lock = threading.Lock()

        # F08: navigation version counter — incremented on every navigate() call
        # and on every main-frame Page.frameNavigated event.  Used as part of
        # the AX snapshot cache key so that any navigation automatically
        # invalidates cached snapshots without requiring callbacks.
        self._nav_version: int = 0

        # F03: event queue for unit-test compatibility (test 10 checks this)
        self._event_queue: queue.Queue = queue.Queue()

        # F03: reader thread (started by _start_page_listener via open())
        self._reader_thread: threading.Thread | None = None
        self._reader_running = False

        # F03: listener thread alias (same as _reader_thread — for test compat)
        self._listener_thread: threading.Thread | None = None
        self._listener_running = False

        # F01: console log capture
        self._console_enabled: bool = False
        self._console_logs: list[dict] = []
        self._console_lock = threading.Lock()
        self._MAX_CONSOLE_ENTRIES = 500

        # F05: performance metrics
        self._performance_enabled: bool = False

        # F02: network trace
        self._network_enabled: bool = False
        # keyed by requestId — stores in-progress and completed entries
        self._network_requests: dict[str, dict] = {}
        self._network_lock = threading.Lock()
        self._MAX_NETWORK_ENTRIES = 200

        # F09: network stream watchers
        # watch_requests(pattern) subscribes to real-time network events.
        # Each entry: (url_pattern, queue). The queue receives tuples:
        #   ("request", request_id, url)   — request fired
        #   ("data",    request_id, bytes) — chunk received
        #   ("done",    request_id, None)  — stream complete
        #   ("error",   request_id, text)  — load failed
        # Sentinel None is sent when watcher is removed.
        self._net_watchers: list = []  # list of (pattern: str, q: queue.Queue)
        self._net_watchers_lock = threading.Lock()

        # F09-fix: active stream URL map — NOT size-limited.
        # _network_requests evicts entries at 200, so data/done/error events lose
        # the URL when the entry is evicted. This dict maps req_id → url for ALL
        # in-flight requests, cleared only on done/error. Watchers always see the URL.
        self._active_streams: dict = {}  # req_id → url

    # ------------------------------------------------------------------
    # Factory
    # ------------------------------------------------------------------

    @classmethod
    def open(cls, port: int) -> "ChromeTab":
        """
        Open a new tab on an existing Chrome instance.

        Uses PUT /json/new (GET returns 405 — fixed vs V3).
        Requires Chrome to be running and the port to be open.
        """
        from tools.v4.chrome_process import open_new_tab, _validate_port  # lazy import

        _validate_port(port)
        tab_info = open_new_tab(port)
        ws_url = tab_info.get("webSocketDebuggerUrl", "")
        if not ws_url:
            raise RuntimeError(f"No webSocketDebuggerUrl in /json/new response: {tab_info}")
        if not ws_url.startswith("ws://127.0.0.1:"):
            raise ValueError(f"webSocketDebuggerUrl must be loopback ws://127.0.0.1:, got: {ws_url}")

        import websockets.sync.client as _ws_sync  # lazy import — not needed in tests

        ws = _ws_sync.connect(ws_url, max_size=10_000_000, ping_interval=None)
        tab_id = tab_info.get("id", "")
        tab = cls(ws=ws, tab_id=tab_id, port=port)
        tab._start_page_listener()
        return tab

    @classmethod
    def attach(cls, ws_url: str, tab_id: str, port: int) -> "ChromeTab":
        """
        Attach to an already-open Chrome tab via its WebSocket debugger URL.

        Does NOT open a new tab — connects to a tab that already exists.
        Useful for controlling a live logged-in session without disturbing it.

        Parameters
        ----------
        ws_url:  webSocketDebuggerUrl from GET /json/list (must be ws://127.0.0.1:...)
        tab_id:  Chrome target id from /json/list
        port:    Chrome remote-debugging port (needed for tab.close() later)

        Example::

            import urllib.request, json
            tabs = json.loads(urllib.request.urlopen("http://localhost:65315/json/list").read())
            t = next(t for t in tabs if "linkedin" in t["url"])
            tab = ChromeTab.attach(t["webSocketDebuggerUrl"], t["id"], 65315)
            print(tab.current_url())
        """
        if not ws_url.startswith("ws://127.0.0.1:") and not ws_url.startswith("ws://localhost:"):
            raise ValueError(f"webSocketDebuggerUrl must be loopback, got: {ws_url}")

        import websockets.sync.client as _ws_sync

        ws = _ws_sync.connect(ws_url, max_size=10_000_000, ping_interval=None)
        tab = cls(ws=ws, tab_id=tab_id, port=port)
        tab._is_attached = True  # close() must not destroy this tab
        tab._start_page_listener()
        return tab

    # ------------------------------------------------------------------
    # Background reader thread (F03)
    # ------------------------------------------------------------------

    def _start_page_listener(self) -> None:
        """
        Enable CDP Page events and start the background reader thread.

        Called at the end of ChromeTab.open(). The reader thread owns all
        ws.recv() calls. send() registers a per-request queue and blocks
        on it; the reader routes each CDP message to the matching queue or
        to the page-event handler.
        """
        # Enable Page and Network domains before starting reader
        self._send_sync("Page.enable", {})
        self._send_sync("Network.enable", {})
        self._network_enabled = True
        self._reader_running = True
        self._listener_running = True  # alias
        t = threading.Thread(
            target=self._reader_loop,
            name=f"chrome-tab-reader-{self._tab_id[:8] if self._tab_id else 'unknown'}",
            daemon=True,
        )
        t.start()
        self._reader_thread = t
        self._listener_thread = t  # alias for close()

        # LOW: register atexit so unclean shutdown (SIGTERM, Ctrl+C mid-send)
        # doesn't block for 30s waiting on an empty response queue.
        import atexit
        atexit.register(self.close)

    def _send_sync(self, method: str, params: dict) -> dict:
        """
        Low-level send that reads directly from ws.recv().

        Only safe to call before the reader thread is running (i.e., during
        _start_page_listener itself). After the reader thread starts, use
        send() instead.
        """
        # SECURITY: guard against concurrent ws.recv() with the reader thread.
        assert not self._reader_running, (
            "_send_sync must not be called after the reader thread has started. "
            "Use send() instead."
        )
        self._id_counter += 1
        msg_id = self._id_counter
        payload = json.dumps({"id": msg_id, "method": method, "params": params})
        self._ws.send(payload)
        deadline = time.monotonic() + self._SEND_TIMEOUT
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"No response for {method} (id={msg_id}) within {self._SEND_TIMEOUT}s")
            raw = self._ws.recv(timeout=remaining)
            msg = json.loads(raw)
            if msg.get("id") != msg_id:
                # Event that arrived before our response — enqueue
                self._event_queue.put_nowait(msg)
                self._handle_page_event(msg)
                continue
            if "error" in msg:
                raise RuntimeError(f"CDP error for {method}: {msg['error']}")
            return msg.get("result", {})

    def _reader_loop(self) -> None:
        """
        Background thread: own all ws.recv() calls.

        Routes each incoming message to either:
        - The matching per-request queue (if msg has an id we registered)
        - The page-event handler (CDP events without an id)
        """
        while self._reader_running:
            try:
                raw = self._ws.recv(timeout=0.5)
            except TimeoutError:
                continue
            except Exception:
                # WS closed or error — stop
                break
            try:
                msg = json.loads(raw)
            except Exception:
                continue

            msg_id = msg.get("id")
            if msg_id is not None:
                # Command response — deliver to the waiting send() call
                with self._rq_lock:
                    q = self._response_queues.get(msg_id)
                if q is not None:
                    q.put_nowait(msg)
                # else: unsolicited response with id, ignore
            else:
                # CDP event — enqueue for test compat and handle
                self._event_queue.put_nowait(msg)
                self._handle_page_event(msg)

    # ------------------------------------------------------------------
    # Core CDP send
    # ------------------------------------------------------------------

    def send(self, method: str, params: dict | None = None, timeout: float | None = None) -> dict:
        """
        Send a CDP command and wait for the matching response.

        Thread-safe via this tab's own _lock (not shared with other tabs).

        timeout: override the default _SEND_TIMEOUT for this call (e.g. for
                 Runtime.evaluate with awaitPromise=True on long-running Promises).

        When the reader thread is running, registers a per-request queue and
        blocks on it. The reader thread delivers the response.

        When the reader thread is NOT running (e.g. in unit tests that never
        call _start_page_listener), falls back to direct ws.recv() with event
        routing to _event_queue — preserving test compatibility.
        """
        effective_timeout = timeout if timeout is not None else self._SEND_TIMEOUT
        with self._lock:
            self._id_counter += 1
            msg_id = self._id_counter
            payload = json.dumps({"id": msg_id, "method": method, "params": params or {}})

            if self._reader_running:
                # F03: reader thread owns ws.recv() — use per-request queue
                resp_q: queue.Queue = queue.Queue()
                with self._rq_lock:
                    self._response_queues[msg_id] = resp_q
                try:
                    self._ws.send(payload)
                    try:
                        msg = resp_q.get(timeout=effective_timeout)
                    except queue.Empty:
                        raise TimeoutError(
                            f"No response for {method} (id={msg_id}) within {effective_timeout}s"
                        )
                    if "error" in msg:
                        raise RuntimeError(f"CDP error for {method}: {msg['error']}")
                    return msg.get("result", {})
                finally:
                    with self._rq_lock:
                        self._response_queues.pop(msg_id, None)
            else:
                # Fallback: direct recv (reader not started — unit test path)
                self._ws.send(payload)
                deadline = time.monotonic() + effective_timeout
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError(
                            f"No response for {method} (id={msg_id}) within {effective_timeout}s"
                        )
                    raw = self._ws.recv(timeout=remaining)
                    msg = json.loads(raw)
                    if msg.get("id") != msg_id:
                        # CDP event or response to a different command — enqueue for listener
                        self._event_queue.put_nowait(msg)
                        continue
                    if "error" in msg:
                        raise RuntimeError(f"CDP error for {method}: {msg['error']}")
                    return msg.get("result", {})

    # ------------------------------------------------------------------
    # JS helpers
    # ------------------------------------------------------------------

    def js(self, expr: str) -> Any:
        """
        Evaluate a JS expression in the page context.

        If expr contains 'return ' (case-sensitive) it is wrapped in an IIFE
        so that bare `return value` statements work without a function wrapper.
        Returns the primitive value or None.
        """
        if "return " in expr:
            expr = f"(function(){{{expr}}})()"
        result = self.send("Runtime.evaluate", {
            "expression": expr,
            "returnByValue": True,
        })
        return result.get("result", {}).get("value")

    def js_await(self, expr: str, timeout_ms: int = 30000) -> Any:
        """
        Evaluate a JS expression that returns a Promise and block until it resolves.

        Uses Runtime.evaluate with awaitPromise=True. Enables event-driven waiting
        instead of polling: inject a MutationObserver that resolves the Promise when
        the desired DOM state is reached.

        timeout_ms: CDP-level timeout for the expression itself (default 30s).
        The send() call uses a longer socket timeout to avoid premature abort.
        """
        socket_timeout = (timeout_ms / 1000) + 15  # socket > JS timeout to avoid premature abort
        result = self.send(
            "Runtime.evaluate",
            {
                "expression": expr,
                "returnByValue": True,
                "awaitPromise": True,
            },
            timeout=socket_timeout,
        )
        return result.get("result", {}).get("value")

    # ------------------------------------------------------------------
    # Navigation
    # ------------------------------------------------------------------

    def navigate(self, url: str, wait_s: float = 3.0) -> None:
        """Navigate to url, wait for document.readyState===complete, then wait_s for SPA hydration."""
        self.send("Page.navigate", {"url": url})
        with self._page_lock:
            self._nav_version += 1
        # Wait for DOM ready (up to 15s) instead of a fixed sleep
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            try:
                state = self.js("return document.readyState")
                if state == "complete":
                    break
            except Exception:
                pass
            time.sleep(0.2)
        # SPA hydration buffer (reduced: DOM is ready, just needs JS to wire up)
        if wait_s > 0:
            time.sleep(min(wait_s, 2.0))

    # ------------------------------------------------------------------
    # Wait helpers
    # ------------------------------------------------------------------

    def wait_last(self, selector: str, timeout_s: float = 10.0) -> str | None:
        """
        Poll until selector matches at least one element.

        Returns the LAST matched element's innerText.

        V3 bug fix: V3 used querySelector which returns the FIRST element.
        V4 uses querySelectorAll + els[els.length-1] to return the LAST,
        which gives "most recent message" semantics in chat interfaces.
        """
        # Use send() directly — bypasses js() IIFE detection to avoid double-wrapping.
        js_expr = (
            f"(()=>{{var els=document.querySelectorAll({json.dumps(selector)});"
            f"var el=els[els.length-1];return el?el.innerText.trim():null;}})()"
        )
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            result = self.send("Runtime.evaluate", {"expression": js_expr, "returnByValue": True})
            value = result.get("result", {}).get("value")
            if value is not None:
                return value
            time.sleep(0.2)
        return None

    # ------------------------------------------------------------------
    # Health check
    # ------------------------------------------------------------------

    def ping(self) -> bool:
        """
        Health check: evaluate the integer literal 1.

        Returns True if the WebSocket and JS engine are responsive.
        Returns False on any exception (WS closed, timeout, etc.).

        V3 had no equivalent — a dead WS was undetectable.
        """
        try:
            result = self.js("1")
            return result == 1
        except Exception:
            return False

    # ------------------------------------------------------------------
    # F03: URL state & navigation history
    # ------------------------------------------------------------------

    def _handle_page_event(self, msg: dict) -> None:
        """Update URL/title caches from CDP Page events."""
        method = msg.get("method", "")
        if method == "Page.frameNavigated":
            frame = msg.get("params", {}).get("frame", {})
            # Only track the main frame — sub-frames (iframes, ads, tracking pixels)
            # have a parentId. Without this filter, LinkedIn's ad iframes pollute history.
            if frame.get("parentId"):
                return
            url = frame.get("url", "")
            if url and url != "about:blank":
                with self._page_lock:
                    self._current_url = url
                    self._nav_version += 1  # F08: invalidate AX snapshot cache
                    if len(self._history) >= 50:
                        self._history.pop(0)
                    self._history.append(url)
        elif method == "Page.loadEventFired":
            # Invalidate title cache — will be fetched lazily on next page_title() call
            with self._page_lock:
                self._page_title = None

        elif method == "Runtime.consoleAPICalled":
            if not self._console_enabled:
                return
            params = msg.get("params", {})
            level = params.get("type", "log")  # log/info/warning/error/debug
            args = params.get("args", [])
            text_parts = []
            for arg in args:
                if arg.get("type") == "string":
                    text_parts.append(arg.get("value", ""))
                elif "description" in arg:
                    text_parts.append(arg["description"])
                else:
                    text_parts.append(str(arg.get("value", "")))
            text = " ".join(text_parts)
            timestamp = params.get("timestamp", 0.0)
            source = params.get("stackTrace", {}).get("callFrames", [{}])[0].get("url", "")
            entry = {"level": level, "text": text, "timestamp": timestamp, "source": source}
            with self._console_lock:
                if len(self._console_logs) >= self._MAX_CONSOLE_ENTRIES:
                    self._console_logs.pop(0)
                self._console_logs.append(entry)

        elif method == "Runtime.exceptionThrown":
            if not self._console_enabled:
                return
            params = msg.get("params", {})
            detail = params.get("exceptionDetails", {})
            text = detail.get("text", "")
            exc = detail.get("exception", {})
            if exc.get("description"):
                text = exc["description"]
            timestamp = params.get("timestamp", 0.0)
            entry = {"level": "error", "text": text, "timestamp": timestamp, "source": "exception"}
            with self._console_lock:
                if len(self._console_logs) >= self._MAX_CONSOLE_ENTRIES:
                    self._console_logs.pop(0)
                self._console_logs.append(entry)

        # F02: network trace events
        elif method == "Network.requestWillBeSent":
            if not self._network_enabled:
                return
            params = msg.get("params", {})
            req_id = params.get("requestId", "")
            if not req_id:
                return
            request = params.get("request", {})
            entry = {
                "request_id": req_id,
                "url": request.get("url", ""),
                "method": request.get("method", "GET"),
                "status": None,
                "status_text": "",
                "duration_ms": None,
                "encoded_data_length": None,
                "error": None,
                "timestamp": params.get("timestamp", time.monotonic()),
            }
            with self._network_lock:
                # Evict oldest entry if buffer full
                if len(self._network_requests) >= self._MAX_NETWORK_ENTRIES:
                    oldest_key = next(iter(self._network_requests))
                    self._network_requests.pop(oldest_key)
                self._network_requests[req_id] = entry
                # F09-fix: track URL in eviction-free map for watcher URL lookups
                self._active_streams[req_id] = entry["url"]
            # F09: notify watchers
            self._notify_watchers("request", req_id, entry["url"], None)

        elif method == "Network.responseReceived":
            if not self._network_enabled:
                return
            params = msg.get("params", {})
            req_id = params.get("requestId", "")
            response = params.get("response", {})
            with self._network_lock:
                entry = self._network_requests.get(req_id)
                if entry is not None:
                    entry["status"] = response.get("status")
                    entry["status_text"] = response.get("statusText", "")
                    entry["encoded_data_length"] = response.get("encodedDataLength")
                    # duration_ms: diff between response timestamp and request timestamp
                    resp_ts = params.get("timestamp", 0.0)
                    if resp_ts and entry["timestamp"]:
                        entry["duration_ms"] = (resp_ts - entry["timestamp"]) * 1000.0

        elif method == "Network.dataReceived":
            if not self._network_enabled:
                return
            params = msg.get("params", {})
            req_id = params.get("requestId", "")
            data_length = params.get("dataLength", 0)
            with self._network_lock:
                # F09-fix: use _active_streams (eviction-free) for URL lookup
                url = self._active_streams.get(req_id, "")
                if not url:
                    entry = self._network_requests.get(req_id, {})
                    url = entry.get("url", "")
            self._notify_watchers("data", req_id, url, data_length)

        elif method == "Network.loadingFinished":
            if not self._network_enabled:
                return
            params = msg.get("params", {})
            req_id = params.get("requestId", "")
            with self._network_lock:
                entry = self._network_requests.get(req_id)
                if entry is not None:
                    entry["completed"] = True
                    ts = params.get("timestamp")
                    if ts and entry.get("timestamp"):
                        entry["duration_ms"] = (ts - entry["timestamp"]) * 1000.0
                # F09-fix: use _active_streams for URL, then clean up
                url = self._active_streams.pop(req_id, None) or (entry or {}).get("url", "")
            self._notify_watchers("done", req_id, url, None)

        elif method == "Network.loadingFailed":
            if not self._network_enabled:
                return
            params = msg.get("params", {})
            req_id = params.get("requestId", "")
            error_text = params.get("errorText", "Unknown error")
            with self._network_lock:
                entry = self._network_requests.get(req_id)
                if entry is not None:
                    entry["error"] = error_text
                # F09-fix: use _active_streams for URL, then clean up
                url = self._active_streams.pop(req_id, None) or (entry or {}).get("url", "")
            self._notify_watchers("error", req_id, url, error_text)

    # ------------------------------------------------------------------
    # F09: network stream watchers (event-driven, no polling)
    # ------------------------------------------------------------------

    def _notify_watchers(self, event: str, req_id: str, url: str, data: Any) -> None:
        """Deliver a network event to all registered watchers matching the URL."""
        import logging as _logging
        _log = _logging.getLogger("neo.tab.watchers")
        with self._net_watchers_lock:
            watchers = list(self._net_watchers)
        if watchers and "conversation" in url:
            _log.debug("[notify] event=%s req_id=%s url=%s watchers=%d",
                       event, req_id[:8] if req_id else "?", url[:100], len(watchers))
        for pattern, q in watchers:
            if pattern in url:
                try:
                    q.put_nowait((event, req_id, url, data))
                except Exception:
                    pass

    def watch_requests(self, url_pattern: str) -> "queue.Queue":
        """
        Subscribe to real-time network events for requests whose URL contains url_pattern.

        Returns a queue that receives tuples:
          ("request", request_id, url, None)    — request fired
          ("data",    request_id, url, length)  — data chunk received (length in bytes)
          ("done",    request_id, url, None)    — stream/load complete
          ("error",   request_id, url, text)    — load failed

        Call unwatch_requests(q) when done. Not calling it leaks the entry.
        Network monitoring is always on (enabled at tab open).
        """
        q: queue.Queue = queue.Queue()
        with self._net_watchers_lock:
            self._net_watchers.append((url_pattern, q))
        return q

    def unwatch_requests(self, q: "queue.Queue") -> None:
        """Unregister a watcher queue returned by watch_requests()."""
        with self._net_watchers_lock:
            self._net_watchers = [(p, lq) for p, lq in self._net_watchers if lq is not q]


    def current_url(self) -> str:
        """
        Return the current URL of the tab.

        Uses _current_url populated by Page.frameNavigated listener.
        Falls back to JS if not yet cached. Returns "" if never navigated.
        """
        with self._page_lock:
            if self._current_url is not None:
                return self._current_url
        # Fallback: JS call (also skips about:blank — return "" for blank tabs)
        val = self.js("return window.location.href")
        if val and val != "about:blank":
            with self._page_lock:
                self._current_url = val
            return val
        return ""

    def page_title(self) -> str:
        """
        Return the page title.

        Cached after each load. Falls back to JS if not cached. Returns "".
        """
        with self._page_lock:
            if self._page_title is not None:
                return self._page_title
        val = self.js("return document.title") or ""
        with self._page_lock:
            self._page_title = val
        return val

    def navigation_history(self) -> list[str]:
        """
        Return visited URLs in chronological order (max 50).

        Only includes real URLs (not about:blank). Thread-safe.
        """
        with self._page_lock:
            return list(self._history)

    def is_at(self, url: str) -> bool:
        """Return True if current_url() matches url exactly."""
        return self.current_url() == url

    # ------------------------------------------------------------------
    # F01: Console log capture
    # ------------------------------------------------------------------

    def enable_console(self) -> None:
        """
        Enable console log capture via CDP Runtime domain.
        Idempotent — safe to call multiple times.
        Must be called before navigating to capture logs from page load.
        """
        if not self._console_enabled:
            self.send("Runtime.enable", {})
            self._console_enabled = True

    def get_console_logs(self) -> list[dict]:
        """
        Return all captured console messages since last clear (or since enable).
        Returns copies — thread-safe snapshot. Mutations to the returned list
        or its dicts do not affect the internal buffer.
        Each entry: {"level": str, "text": str, "timestamp": float, "source": str}
        level: "log" | "info" | "warning" | "error" | "debug"
        """
        with self._console_lock:
            return [dict(entry) for entry in self._console_logs]

    def clear_console_logs(self) -> None:
        """Clear the console log buffer."""
        with self._console_lock:
            self._console_logs.clear()

    # ------------------------------------------------------------------
    # F02: Network trace
    # ------------------------------------------------------------------

    def enable_network(self) -> None:
        """
        Enable network request capture via CDP Network domain.
        Idempotent — safe to call multiple times.
        Must be called before navigating to capture all requests.
        """
        if not self._network_enabled:
            self.send("Network.enable", {})
            self._network_enabled = True

    def get_network_requests(self) -> list[dict]:
        """
        Return all captured network requests, sorted by timestamp (ascending).
        Returns deep copies — mutations do not affect the internal buffer.
        Each entry:
          {request_id, url, method, status, status_text, duration_ms,
           encoded_data_length, error, timestamp}
        status / duration_ms / encoded_data_length may be None if no response received.
        error is set (str) on Network.loadingFailed events.
        """
        with self._network_lock:
            entries = [dict(e) for e in self._network_requests.values()]
        entries.sort(key=lambda e: e["timestamp"])
        return entries

    def get_network_request(self, url_pattern: str) -> dict | None:
        """
        Return the first captured request whose URL contains url_pattern.
        Returns None if not found.
        """
        with self._network_lock:
            for entry in self._network_requests.values():
                if url_pattern in entry.get("url", ""):
                    return dict(entry)
        return None

    def clear_network_log(self) -> None:
        """
        Clear the network request buffer.
        Does NOT disable network capture — enable_network() state is preserved.
        """
        with self._network_lock:
            self._network_requests.clear()

    # ------------------------------------------------------------------
    # F04: Screenshot
    # ------------------------------------------------------------------

    _VALID_SCREENSHOT_FORMATS = frozenset({"png", "jpeg"})

    def screenshot(self, format: str = "png", quality: int = 80) -> bytes:
        """
        Capture the current viewport as image bytes.

        format: "png" (lossless) | "jpeg" (lossy, smaller)
        quality: 0-100, only meaningful for jpeg.
        Returns raw bytes. Raises ValueError for unsupported format.
        """
        import base64
        if format not in self._VALID_SCREENSHOT_FORMATS:
            raise ValueError(
                f"Unsupported screenshot format {format!r}. "
                f"Use one of: {sorted(self._VALID_SCREENSHOT_FORMATS)}"
            )
        params: dict = {"format": format}
        if format == "jpeg":
            params["quality"] = max(0, min(100, quality))
        result = self.send("Page.captureScreenshot", params)
        data = result.get("data", "")
        return base64.b64decode(data)

    def screenshot_base64(self, format: str = "png", quality: int = 80) -> str:
        """
        Capture the current viewport as a base64-encoded string.

        Useful for embedding in JSON payloads or LLM vision inputs.
        Same format/quality semantics as screenshot().
        """
        import base64
        raw = self.screenshot(format=format, quality=quality)
        return base64.b64encode(raw).decode("ascii")

    def screenshot_save(
        self,
        path: "str | Any",
        format: str = "png",
        quality: int = 80,
        base_dir: "Any | None" = None,
    ) -> "Any":
        """
        Capture the viewport and save to disk.

        If base_dir is provided, path must resolve to a location under base_dir
        (path traversal guard). If base_dir is None, any path is accepted — callers
        should pass base_dir when path is user/LLM-controlled.

        Creates parent directories if they don't exist.
        Returns the resolved Path.
        """
        from pathlib import Path as _Path
        dest = _Path(path).resolve()
        if base_dir is not None:
            safe_base = _Path(base_dir).resolve()
            if not dest.is_relative_to(safe_base):
                raise ValueError(
                    f"screenshot_save path {dest!r} is outside allowed base {safe_base!r}"
                )
        dest.parent.mkdir(parents=True, exist_ok=True)
        raw = self.screenshot(format=format, quality=quality)
        dest.write_bytes(raw)
        return dest

    # ------------------------------------------------------------------
    # F05: Performance Metrics
    # ------------------------------------------------------------------

    def enable_performance(self) -> None:
        """
        Enable the CDP Performance domain.
        Idempotent — safe to call multiple times.
        Must be called before get_metrics() to receive data.
        """
        if not self._performance_enabled:
            self.send("Performance.enable", {})
            self._performance_enabled = True

    def get_metrics(self) -> dict:
        """
        Return current performance metrics as a flat dict {name: value}.

        Calls CDP Performance.getMetrics. Returns {} if performance domain
        has not been enabled.

        Common keys: JSHeapUsedSize, JSHeapTotalSize, Nodes, Documents,
        Frames, LayoutCount, RecalcStyleCount, TaskDuration.
        """
        if not self._performance_enabled:
            return {}
        result = self.send("Performance.getMetrics", {})
        metrics_list = result.get("metrics", [])
        return {m["name"]: m["value"] for m in metrics_list if "name" in m and "value" in m}

    def get_metric(self, name: str) -> "float | None":
        """
        Return a single performance metric by name.
        Returns None if not found or performance not enabled.
        """
        return self.get_metrics().get(name)

    # ------------------------------------------------------------------
    # DOM interaction
    # ------------------------------------------------------------------

    def click(self, selector: str) -> bool:
        """
        Click the first element matching selector.

        Returns True if the element was found and clicked, False otherwise.
        Uses CDP Runtime.evaluate — no mouse simulation needed for basic clicks.
        """
        expr = (
            f"(()=>{{var el=document.querySelector({json.dumps(selector)});"
            f"if(el){{el.click();return true;}}return false;}})()"
        )
        result = self.send("Runtime.evaluate", {"expression": expr, "returnByValue": True})
        return result.get("result", {}).get("value") is True

    def wait_for_selector(self, selector: str, timeout_s: float = 10.0) -> bool:
        """
        Poll until selector exists in the DOM.

        Returns True if found within timeout_s, False otherwise.
        Useful for SPA navigation where elements appear asynchronously.
        """
        expr = f"!!document.querySelector({json.dumps(selector)})"
        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            result = self.send("Runtime.evaluate", {"expression": expr, "returnByValue": True})
            if result.get("result", {}).get("value") is True:
                return True
            time.sleep(0.2)
        return False

    # ------------------------------------------------------------------
    # Cookie management (V3 fix: re-sync any time, not only at startup)
    # ------------------------------------------------------------------

    def set_cookies(self, cookies: list[dict]) -> None:
        """
        Inject cookies into the browser via CDP Network.setCookies.

        V3 bug: cookies were synced once at startup from a Chromium profile
        copy. V4: call this method any time — before navigation, mid-session,
        or after re-auth.

        Each cookie dict follows CDP Network.CookieParam schema:
        {"name": str, "value": str, "domain": str, "path": str, ...}
        """
        self.send("Network.setCookies", {"cookies": cookies})

    def get_cookies(self, url: str | None = None) -> list[dict]:
        """
        Retrieve cookies from the browser via CDP Network.getCookies.

        If url is provided, returns only cookies applicable to that URL.
        Returns a list of CDP Network.Cookie dicts.
        """
        params: dict = {}
        if url is not None:
            params["urls"] = [url]
        result = self.send("Network.getCookies", params)
        return result.get("cookies", [])

    # ------------------------------------------------------------------
    # Teardown
    # ------------------------------------------------------------------

    def __enter__(self) -> "ChromeTab":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def close(self) -> None:
        """
        Close WebSocket connection and remove the tab from Chrome.

        Tab removal uses GET /json/close/{id} per the CDP HTTP API.
        Errors are suppressed — close() should never raise.
        """
        # F03: stop reader/listener thread before closing WS
        self._reader_running = False
        self._listener_running = False
        if self._reader_thread and self._reader_thread.is_alive():
            self._reader_thread.join(timeout=2.0)

        try:
            self._ws.close()
        except Exception:
            pass

        # Only destroy the Chrome tab if WE opened it (not attached to someone else's).
        if self._tab_id and not self._is_attached:
            try:
                from tools.v4.chrome_process import _validate_port
                _validate_port(self._port)
                url = f"http://127.0.0.1:{self._port}/json/close/{self._tab_id}"
                req = urllib.request.Request(url, method="GET")
                urllib.request.urlopen(req, timeout=3.0).close()
            except Exception:
                pass
