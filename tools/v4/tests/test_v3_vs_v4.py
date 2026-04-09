"""
V3 vs V4 comparative notes:

T0 — ChromeProcess
- V3: find_free_port → inline socket code inside chrome() function (~line 1217)
  V4: find_free_port() → dedicated, testeable function in chrome_process.py

- V3: is_alive → never checked; chrome() returns a zombie GhostChrome object if Chrome died
  V4: ChromeProcess.health_check() → explicit boolean, callers verify before use

- V3: /json/new → GET request → HTTP 405 Method Not Allowed (Chrome requires PUT)
  V4: open_new_tab() → PUT request (correct per CDP spec)

- V3: kills PIDs from a shared PID file on startup → can kill sibling processes or
      unrelated services if the file was written by a different instance
  V4: ChromeProcess.kill() → only sends signals to self.pid (the pid it launched),
      no shared PID file, no risk of cross-process kill

- V3: _load_watchers() called at module import time → runs before Chrome is ready,
      can cause startup failures or race conditions
  V4: No code runs at import time; all initialization is explicit

- V3: port file goes stale after restart → chrome() may connect to wrong port
  V4: ChromeProcess stores .port as instance attribute; no file-based port discovery

T1 — ChromeTab
- V3: single _recv_lock (RLock) shared across ALL tabs (~line 804)
      → if Tab A is waiting for a CDP response, Tab B's send() is completely blocked
  V4: each ChromeTab instance owns its own _lock — fully independent I/O

- V3: GhostChrome.ws property returns self._tabs[self._active] (~line 808-809)
      → _active can be changed by any thread between the read and the send
      → race condition: Thread A switches to 'toni', Thread B switches to 'default',
         Thread A calls ws.send() → message goes on wrong WebSocket
  V4: ChromeTab owns exactly one WebSocket; there is no _active indirection

- V3: wait_selector uses querySelector → returns FIRST matched element
  V4: wait_last() uses querySelectorAll + els[els.length-1] → returns LAST matched element
      Critical for chat UIs: last element = most recent message

- V3: no WebSocket health check; a dead WS is silently ignored or raises an unexpected exception
  V4: ChromeTab.ping() → evaluate '1', returns True/False for explicit health checking
"""
# ---------------------------------------------------------------------------
# T1 comparative tests
# ---------------------------------------------------------------------------
from __future__ import annotations

import json
import threading
from unittest.mock import MagicMock

from tools.v4.chrome_tab import ChromeTab


def _make_tab(tab_id: str = "tab") -> ChromeTab:
    ws = MagicMock()
    return ChromeTab(ws=ws, tab_id=tab_id, port=9222)


class TestV3VsV4ChromeTab:
    """Executable documentation of T1 bug fixes."""

    def test_each_tab_has_own_lock(self):
        """V3: single _recv_lock for all tabs. V4: each ChromeTab has own _lock."""
        tab1 = _make_tab("t1")
        tab2 = _make_tab("t2")
        assert tab1._lock is not tab2._lock, (
            "V4 fix: each ChromeTab must own a distinct lock, not share one across all tabs"
        )

    def test_querySelectorAll_not_querySelector(self):
        """V3: querySelector (first). V4: querySelectorAll + last element."""
        tab = _make_tab()
        # Capture the JS expression passed to send() by wait_last()
        captured: list[str] = []

        def capture_send(method: str, params: dict | None = None) -> dict:
            if params and "expression" in params:
                captured.append(params["expression"])
            return {"result": {"value": "msg"}}

        tab.send = capture_send  # type: ignore[method-assign]
        tab.wait_last(".bubble", timeout_s=1.0)

        assert captured, "wait_last must call send() with an expression"
        assert "querySelectorAll" in captured[0], (
            "V4 fix: must use querySelectorAll (not querySelector) to reach the last element"
        )
        # Make sure plain querySelector isn't there (querySelectorAll is fine)
        plain = captured[0].replace("querySelectorAll", "")
        assert "querySelector" not in plain

    def test_ping_method_exists_and_returns_bool(self):
        """V3: no WebSocket health check. V4: ping() for explicit liveness detection."""
        tab = _make_tab()
        tab.js = lambda expr: 1  # type: ignore[method-assign]
        result = tab.ping()
        assert isinstance(result, bool), "ping() must return a bool"
        assert result is True
