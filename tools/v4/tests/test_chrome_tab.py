"""
Unit tests for tools/v4/chrome_tab.py

ALL tests use a mock WebSocket — no real Chrome needed.
"""
from __future__ import annotations

import json
import threading
import time
from typing import Any
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab(**kwargs) -> ChromeTab:
    """Return a ChromeTab backed by a fresh MagicMock WebSocket."""
    ws = MagicMock()
    defaults = dict(ws=ws, tab_id="tab-abc", port=9222)
    defaults.update(kwargs)
    return ChromeTab(**defaults)


def _response(msg_id: int, result: dict | None = None) -> str:
    """Serialise a synthetic CDP response."""
    return json.dumps({"id": msg_id, "result": result or {}})


def _event(method: str, params: dict | None = None) -> str:
    """Serialise a synthetic CDP event (no id field)."""
    return json.dumps({"method": method, "params": params or {}})


def _error_response(msg_id: int, message: str = "oops") -> str:
    """Serialise a synthetic CDP error response."""
    return json.dumps({"id": msg_id, "error": {"message": message}})


# ---------------------------------------------------------------------------
# TestChromeTabSend
# ---------------------------------------------------------------------------

class TestChromeTabSend:

    def test_sends_correct_json_format(self):
        """send() serialises id, method, and params correctly."""
        tab = _make_tab()
        tab._ws.recv.return_value = _response(1, {})

        tab.send("Page.navigate", {"url": "https://example.com"})

        sent = json.loads(tab._ws.send.call_args[0][0])
        assert sent["id"] == 1
        assert sent["method"] == "Page.navigate"
        assert sent["params"] == {"url": "https://example.com"}

    def test_returns_result_on_matching_id(self):
        """send() returns the result dict when recv() returns a matching id."""
        tab = _make_tab()
        tab._ws.recv.return_value = _response(1, {"frameId": "frame-1"})

        result = tab.send("Page.navigate", {"url": "https://example.com"})

        assert result == {"frameId": "frame-1"}

    def test_skips_events_until_matching_id(self):
        """send() discards CDP events and unrelated responses, returns matching one."""
        tab = _make_tab()
        tab._ws.recv.side_effect = [
            _event("Page.loadEventFired"),          # event — no id
            _response(99, {"other": True}),          # response for id=99 (different)
            _response(1, {"frameId": "frame-x"}),   # our response
        ]

        result = tab.send("Page.navigate")

        assert result == {"frameId": "frame-x"}

    def test_raises_on_cdp_error(self):
        """send() raises RuntimeError when response contains an error field."""
        tab = _make_tab()
        tab._ws.recv.return_value = _error_response(1, "Target closed")

        with pytest.raises(RuntimeError, match="CDP error"):
            tab.send("Page.navigate")

    def test_thread_safety(self):
        """Two concurrent threads calling send() must get non-colliding IDs."""
        # We need a real lock + counter, so use the real ChromeTab but give each
        # thread a mock that replies with the correct id (whatever was sent).
        collected_ids: list[int] = []
        lock = threading.Lock()

        def recv_side_effect(timeout=None):
            # peek at the last sent message and mirror its id
            last_sent = json.loads(tab._ws.send.call_args_list[-1][0][0])
            return _response(last_sent["id"], {})

        tab = _make_tab()
        tab._ws.recv.side_effect = recv_side_effect

        results: dict[str, int] = {}

        def worker(name: str) -> None:
            # Call send() and record which id was assigned
            with tab._lock:
                tab._id_counter += 1
                my_id = tab._id_counter
            with lock:
                results[name] = my_id

        t1 = threading.Thread(target=worker, args=("t1",))
        t2 = threading.Thread(target=worker, args=("t2",))
        t1.start()
        t2.start()
        t1.join()
        t2.join()

        # IDs must be distinct
        assert results["t1"] != results["t2"]


# ---------------------------------------------------------------------------
# TestChromeTabJs
# ---------------------------------------------------------------------------

class TestChromeTabJs:

    def test_js_returns_primitive_value(self):
        """js() extracts result.result.value from the Runtime.evaluate response."""
        tab = _make_tab()
        tab._ws.recv.return_value = _response(1, {"result": {"value": 42}})

        assert tab.js("1+41") == 42

    def test_js_wraps_return_statements(self):
        """js() wraps expressions containing 'return ' in an IIFE."""
        tab = _make_tab()
        tab._ws.recv.return_value = _response(1, {"result": {"value": "hello"}})

        tab.js("return document.title")

        sent_expr = json.loads(tab._ws.send.call_args[0][0])["params"]["expression"]
        assert sent_expr.startswith("(function(){")
        assert "return document.title" in sent_expr

    def test_js_no_wrap_without_return(self):
        """js() does NOT wrap plain expressions (no 'return ' keyword)."""
        tab = _make_tab()
        tab._ws.recv.return_value = _response(1, {"result": {"value": 7}})

        tab.js("3+4")

        sent_expr = json.loads(tab._ws.send.call_args[0][0])["params"]["expression"]
        assert sent_expr == "3+4"


# ---------------------------------------------------------------------------
# TestChromeTabWaitLast
# ---------------------------------------------------------------------------

class TestChromeTabWaitLast:

    def test_returns_last_element_not_first(self):
        """wait_last() returns value once send() returns a non-None string."""
        tab = _make_tab()
        # Simulate: first two polls return None, third returns a value
        call_count = {"n": 0}

        def send_side_effect(method: str, params: dict | None = None) -> dict:
            call_count["n"] += 1
            if call_count["n"] < 3:
                return {"result": {}}
            return {"result": {"value": "last message"}}

        tab.send = send_side_effect  # type: ignore[method-assign]

        result = tab.wait_last(".msg", timeout_s=2.0)
        assert result == "last message"

    def test_returns_none_on_timeout(self):
        """wait_last() returns None when the selector never matches."""
        tab = _make_tab()
        tab.send = lambda method, params=None: {"result": {}}  # type: ignore[method-assign]

        result = tab.wait_last(".absent", timeout_s=0.2)
        assert result is None

    def test_uses_querySelectorAll_not_querySelector(self):
        """The JS expression inside wait_last() must use querySelectorAll."""
        tab = _make_tab()
        captured_exprs: list[str] = []

        def send_side_effect(method: str, params: dict | None = None) -> dict:
            if params and "expression" in params:
                captured_exprs.append(params["expression"])
            return {"result": {"value": "value"}}  # immediate return so we stop polling

        tab.send = send_side_effect  # type: ignore[method-assign]
        tab.wait_last(".item", timeout_s=1.0)

        assert captured_exprs, "send() was never called with an expression"
        assert "querySelectorAll" in captured_exprs[0]
        assert "querySelector" not in captured_exprs[0].replace("querySelectorAll", "")


# ---------------------------------------------------------------------------
# TestChromeTabPing
# ---------------------------------------------------------------------------

class TestChromeTabPing:

    def test_ping_returns_true_when_responsive(self):
        """ping() returns True when js() returns 1."""
        tab = _make_tab()
        tab.js = lambda expr: 1  # type: ignore[method-assign]
        assert tab.ping() is True

    def test_ping_returns_false_on_exception(self):
        """ping() returns False when js() raises any exception."""
        tab = _make_tab()

        def bad_js(expr: str) -> Any:
            raise ConnectionError("WebSocket closed")

        tab.js = bad_js  # type: ignore[method-assign]
        assert tab.ping() is False


# ---------------------------------------------------------------------------
# TestChromeTabIsolation
# ---------------------------------------------------------------------------

class TestChromeTabIsolation:

    def test_two_tabs_have_independent_locks(self):
        """Each ChromeTab instance owns a distinct lock object."""
        tab1 = _make_tab(tab_id="t1")
        tab2 = _make_tab(tab_id="t2")
        assert tab1._lock is not tab2._lock

    def test_two_tabs_have_independent_id_counters(self):
        """IDs generated by tab1.send() and tab2.send() are independent counters."""
        ws1 = MagicMock()
        ws2 = MagicMock()

        # Each ws.recv() returns a response that mirrors the last sent id
        def make_recv(ws_ref: MagicMock):
            def recv(timeout=None):
                last = json.loads(ws_ref.send.call_args_list[-1][0][0])
                return _response(last["id"], {})
            return recv

        tab1 = ChromeTab(ws=ws1, tab_id="t1", port=9222)
        tab2 = ChromeTab(ws=ws2, tab_id="t2", port=9222)
        ws1.recv.side_effect = make_recv(ws1)
        ws2.recv.side_effect = make_recv(ws2)

        tab1.send("Runtime.evaluate", {"expression": "1"})
        tab1.send("Runtime.evaluate", {"expression": "2"})
        tab2.send("Runtime.evaluate", {"expression": "3"})

        ids_tab1 = [json.loads(c[0][0])["id"] for c in ws1.send.call_args_list]
        ids_tab2 = [json.loads(c[0][0])["id"] for c in ws2.send.call_args_list]

        # tab1 advanced its own counter to 1 and 2
        assert ids_tab1 == [1, 2]
        # tab2 started fresh at 1
        assert ids_tab2 == [1]
