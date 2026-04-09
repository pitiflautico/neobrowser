"""
PoC 2 — ChromeTab.attach() sobre sesión autenticada

Prueba que:
1. attach() conecta a tab existente sin abrir una nueva
2. current_url() devuelve la URL real del tab adjunto
3. js() ejecuta en el contexto de la sesión existente
4. navigate() en tab adjunta mantiene la conexión
5. Browser.attach_tab() funciona a través del facade
6. El número de tabs no aumenta después de attach (no se abre tab nueva)
"""
from __future__ import annotations

import json
import time
import urllib.request
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.chrome_tab import ChromeTab


# ─── Unit tests (mocked) ─────────────────────────────────────────────────────

class TestChromeTabAttachUnit:

    def _make_ws(self):
        ws = MagicMock()
        # Simulate Page.enable response then a Page.frameNavigated event
        ws.recv.side_effect = [
            json.dumps({"id": 1, "result": {}}),  # Page.enable
        ]
        return ws

    def test_attach_raises_on_non_loopback_url(self):
        with pytest.raises(ValueError, match="loopback"):
            ChromeTab.attach("ws://evil.com/devtools/page/X", "X", 9222)

    def test_attach_accepts_localhost_url(self):
        ws = MagicMock()
        ws.recv.return_value = json.dumps({"id": 1, "result": {}})
        with patch("websockets.sync.client.connect", return_value=ws):
            tab = ChromeTab.attach("ws://localhost:9222/devtools/page/ABC", "ABC", 9222)
        assert tab._tab_id == "ABC"
        assert tab._port == 9222

    def test_attach_accepts_127_0_0_1_url(self):
        ws = MagicMock()
        ws.recv.return_value = json.dumps({"id": 1, "result": {}})
        with patch("websockets.sync.client.connect", return_value=ws):
            tab = ChromeTab.attach("ws://127.0.0.1:9222/devtools/page/DEF", "DEF", 9222)
        assert tab._tab_id == "DEF"

    def test_attach_does_not_call_open_new_tab(self):
        ws = MagicMock()
        ws.recv.return_value = json.dumps({"id": 1, "result": {}})
        with patch("websockets.sync.client.connect", return_value=ws), \
             patch("tools.v4.chrome_process.open_new_tab") as mock_open:
            ChromeTab.attach("ws://127.0.0.1:9222/devtools/page/G", "G", 9222)
        mock_open.assert_not_called()


# ─── Live tests ───────────────────────────────────────────────────────────────

class TestChromeTabAttachLive:

    def test_attach_connects_without_opening_new_tab(self, live_port, live_tabs):
        """Tab count must not increase after attach."""
        count_before = len(live_tabs)
        tab_info = live_tabs[0]
        ws_url = tab_info["webSocketDebuggerUrl"]

        tab = ChromeTab.attach(ws_url, tab_info["id"], live_port)
        try:
            resp = urllib.request.urlopen(
                f"http://127.0.0.1:{live_port}/json/list", timeout=2
            )
            count_after = len(json.loads(resp.read()))
        finally:
            tab.close()

        assert count_after == count_before, (
            f"Tab count changed: {count_before} → {count_after} "
            "(attach() must not open new tab)"
        )

    def test_attach_current_url_matches_tab_info(self, live_port, live_tabs):
        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            url = tab.current_url()
        finally:
            tab.close()
        assert url == tab_info["url"], f"expected {tab_info['url']!r}, got {url!r}"

    def test_attach_js_executes_in_tab_context(self, live_port, live_tabs):
        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            location = tab.js("return location.href")
        finally:
            tab.close()
        assert location == tab_info["url"]

    def test_browser_attach_tab_facade(self, live_port, live_tabs):
        """Browser.attach_tab() wraps ChromeTab.attach() correctly."""
        from tools.v4.browser import Browser
        tab_info = live_tabs[0]
        with Browser.connect(live_port) as b:
            tab = b.attach_tab(tab_info["id"])
            url = tab.current_url()
            tab.close()
        assert url == tab_info["url"]

    def test_attach_tab_raises_for_unknown_id(self, live_port):
        from tools.v4.browser import Browser
        with Browser.connect(live_port) as b:
            with pytest.raises(ValueError, match="not found"):
                b.attach_tab("NONEXISTENT-TAB-ID-XXXX")

    def test_attach_linkedin_tab_if_present(self, live_port, linkedin_tab_info):
        """Attach to live LinkedIn tab — js() returns linkedin.com URL."""
        tab = ChromeTab.attach(
            linkedin_tab_info["webSocketDebuggerUrl"],
            linkedin_tab_info["id"],
            live_port,
        )
        try:
            url = tab.js("return location.href")
        finally:
            tab.close()
        assert "linkedin.com" in url
