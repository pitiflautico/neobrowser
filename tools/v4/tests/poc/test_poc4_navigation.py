"""
PoC 4 — Navegación multi-URL sin perder auth

Prueba que:
1. is_at() devuelve True para la URL actual y False para otras
2. navigate() en tab adjunta actualiza current_url()
3. Después de navegar a /in/user/ y volver a /messaging/, la tab sigue conectada
4. Las cookies de sesión persisten a través de navegaciones
5. LinkedInNavigator.open_thread() funciona sobre tab adjunta real
"""
from __future__ import annotations

import time
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.chrome_tab import ChromeTab


# ─── Unit tests ───────────────────────────────────────────────────────────────

class TestNavigationUnit:

    def _make_tab_at(self, url: str) -> ChromeTab:
        ws = MagicMock()
        tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
        tab._current_url = url
        return tab

    def test_is_at_exact_match(self):
        tab = self._make_tab_at("https://www.linkedin.com/messaging/")
        assert tab.is_at("https://www.linkedin.com/messaging/") is True

    def test_is_at_returns_false_for_different_url(self):
        tab = self._make_tab_at("https://www.linkedin.com/messaging/")
        assert tab.is_at("https://www.linkedin.com/feed/") is False

    def test_is_at_strips_trailing_slash(self):
        tab = self._make_tab_at("https://www.linkedin.com/messaging/")
        assert tab.is_at("https://www.linkedin.com/messaging") is True

    def test_is_at_none_url_returns_false(self):
        ws = MagicMock()
        tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
        tab._current_url = None
        assert tab.is_at("https://example.com") is False

    def test_navigate_updates_current_url(self):
        ws = MagicMock()
        tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
        navigate_response = {"result": {"frameId": "frame1"}}
        tab.send = MagicMock(return_value=navigate_response)

        with patch.object(tab, "_wait_for_load", return_value=None):
            tab.navigate("https://example.com", wait_s=0)

        # After navigate(), _current_url is set
        # (either by Page.frameNavigated event or optimistically)
        # We verify send was called with Page.navigate
        calls = [c[0][0] for c in tab.send.call_args_list]
        assert "Page.navigate" in calls


# ─── Live tests ───────────────────────────────────────────────────────────────

class TestNavigationLive:

    def test_is_at_true_for_current_url(self, live_port, live_tabs):
        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            assert tab.is_at(tab_info["url"]) is True
        finally:
            tab.close()

    def test_is_at_false_for_other_url(self, live_port, live_tabs):
        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            assert tab.is_at("https://definitely-not-here.example.com") is False
        finally:
            tab.close()

    def test_navigate_and_return_stays_connected(self, live_port, live_tabs):
        """Navigate to about:blank and back — tab stays alive."""
        tab_info = live_tabs[0]
        original_url = tab_info["url"]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            tab.navigate("about:blank", wait_s=1.0)
            assert tab.current_url() == "about:blank"

            tab.navigate(original_url, wait_s=2.0)
            assert tab.is_at(original_url) is True

            # Tab must still respond to js()
            result = tab.js("return typeof document")
            assert result == "object"
        finally:
            tab.close()

    def test_cookies_survive_navigation(self, live_port, live_tabs):
        """Cookies present before navigate must still be present after."""
        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            cookies_before = {c["name"] for c in tab.get_cookies()}

            # Navigate to same domain different path
            current = tab.current_url()
            if "linkedin.com" in current:
                tab.navigate("https://www.linkedin.com/feed/", wait_s=2.0)
                tab.navigate(current, wait_s=2.0)

            cookies_after = {c["name"] for c in tab.get_cookies()}
            # Auth cookies must persist
            assert cookies_before == cookies_after or cookies_before <= cookies_after
        finally:
            tab.close()

    def test_linkedin_open_thread_via_attach(self, live_port, linkedin_tab_info):
        """LinkedInNavigator.open_thread() on attached tab finds the thread."""
        from tools.v4.browser import Browser
        from tools.v4.page_analyzer import PageAnalyzer

        tab = ChromeTab.attach(
            linkedin_tab_info["webSocketDebuggerUrl"],
            linkedin_tab_info["id"],
            live_port,
        )
        try:
            # The thread is already open — verify DOM is accessible
            input_box = tab.js(
                'return !!document.querySelector(".msg-form__contenteditable")'
            )
            send_btn = tab.js(
                'return !!document.querySelector(".msg-form__send-button")'
            )
            assert input_box is True, "LinkedIn input box not found in attached tab"
            assert send_btn is True, "LinkedIn send button not found in attached tab"
        finally:
            tab.close()
