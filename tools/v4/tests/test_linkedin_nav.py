"""
tests/test_linkedin_nav.py

Unit tests for T3: LinkedInNavigator + ChromeTab.click() / wait_for_selector().

All tests are fully mocked — no Chrome or LinkedIn required.
"""
from __future__ import annotations

from unittest.mock import MagicMock, patch, call

import pytest

from tools.v4.chrome_tab import ChromeTab
from tools.v4.linkedin_nav import LinkedInNavigator, _thread_id_from_url


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab(tab_id: str = "t1", port: int = 9222) -> ChromeTab:
    ws = MagicMock()
    return ChromeTab(ws=ws, tab_id=tab_id, port=port)


def _make_analyzer(
    find_by_intent=None,
    click_node_result=True,
    find_input_box=None,
    find_send_button=None,
    type_in_node_result=True,
    find_last_message=None,
):
    """Return a MagicMock analyzer with configurable return values."""
    analyzer = MagicMock()
    analyzer.find_by_intent.return_value = find_by_intent
    analyzer.click_node.return_value = click_node_result
    analyzer.find_input_box.return_value = find_input_box
    analyzer.find_send_button.return_value = find_send_button
    analyzer.type_in_node.return_value = type_in_node_result
    analyzer.find_last_message.return_value = find_last_message
    return analyzer


# ---------------------------------------------------------------------------
# _thread_id_from_url
# ---------------------------------------------------------------------------

class TestThreadIdFromUrl:

    def test_extracts_from_full_url(self):
        url = "https://www.linkedin.com/messaging/thread/2-abc123=="
        assert _thread_id_from_url(url) == "2-abc123=="

    def test_extracts_from_trailing_slash(self):
        url = "https://www.linkedin.com/messaging/thread/2-abc123==/"
        assert _thread_id_from_url(url) == "2-abc123=="

    def test_bare_id_passthrough(self):
        assert _thread_id_from_url("2-abc123==") == "2-abc123=="

    def test_bare_id_with_slash(self):
        assert _thread_id_from_url("/2-abc123==/") == "2-abc123=="


# ---------------------------------------------------------------------------
# ChromeTab.click()
# ---------------------------------------------------------------------------

class TestChromeTabClick:

    def test_click_returns_true_when_element_found(self):
        tab = _make_tab()
        with patch.object(tab, "send", return_value={"result": {"value": True}}) as mock_send:
            result = tab.click(".my-button")
        assert result is True
        mock_send.assert_called_once()
        expr = mock_send.call_args[0][1]["expression"]
        assert "querySelector" in expr
        assert ".my-button" in expr
        assert ".click()" in expr

    def test_click_returns_false_when_element_not_found(self):
        tab = _make_tab()
        with patch.object(tab, "send", return_value={"result": {"value": False}}):
            result = tab.click(".absent")
        assert result is False

    def test_click_returns_false_on_missing_value(self):
        tab = _make_tab()
        with patch.object(tab, "send", return_value={}):
            result = tab.click(".x")
        assert result is False


# ---------------------------------------------------------------------------
# ChromeTab.wait_for_selector()
# ---------------------------------------------------------------------------

class TestChromeTabWaitForSelector:

    def test_returns_true_when_element_appears(self):
        tab = _make_tab()
        call_count = {"n": 0}

        def send_side_effect(method, params=None):
            call_count["n"] += 1
            if call_count["n"] < 3:
                return {"result": {"value": False}}
            return {"result": {"value": True}}

        with patch.object(tab, "send", side_effect=send_side_effect):
            result = tab.wait_for_selector(".item", timeout_s=2.0)

        assert result is True

    def test_returns_false_on_timeout(self):
        tab = _make_tab()
        with patch.object(tab, "send", return_value={"result": {"value": False}}):
            result = tab.wait_for_selector(".never", timeout_s=0.2)
        assert result is False

    def test_expression_uses_querySelector(self):
        tab = _make_tab()
        captured = []

        def send_side_effect(method, params=None):
            if params and "expression" in params:
                captured.append(params["expression"])
            return {"result": {"value": True}}

        with patch.object(tab, "send", side_effect=send_side_effect):
            tab.wait_for_selector(".target", timeout_s=1.0)

        assert captured
        assert "querySelector" in captured[0]
        assert ".target" in captured[0]


# ---------------------------------------------------------------------------
# LinkedInNavigator.open_thread()
# ---------------------------------------------------------------------------

class TestLinkedInNavigatorOpenThread:

    def _make_nav_tab(self, sidebar_found=True, msgs_found=True):
        """Build a tab mock with configurable behaviour for each nav step."""
        tab = _make_tab()
        tab.navigate = MagicMock()
        tab.wait_for_selector = MagicMock(side_effect=[
            sidebar_found,   # first call: sidebar loaded?
            msgs_found,      # second call: message list appeared?
        ])
        tab.js = MagicMock()
        return tab

    def test_open_thread_happy_path(self):
        tab = self._make_nav_tab()
        analyzer = _make_analyzer(find_by_intent=42)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.open_thread(tab, "2-abc123==")

        assert result is True
        tab.navigate.assert_called_once_with(
            "https://www.linkedin.com/messaging/", wait_s=2.0
        )
        # sidebar selector is the fixed one (Bug 1)
        first_sel = tab.wait_for_selector.call_args_list[0][0][0]
        assert first_sel == ".msg-conversations-container__conversations-list"
        # PageAnalyzer.find_by_intent was called
        analyzer.find_by_intent.assert_called_once()
        # click_node called with node returned by find_by_intent
        analyzer.click_node.assert_called_once_with(tab, 42)

    def test_open_thread_with_full_url(self):
        tab = self._make_nav_tab()
        analyzer = _make_analyzer(find_by_intent=99)
        nav = LinkedInNavigator(analyzer=analyzer)
        result = nav.open_thread(tab, "https://www.linkedin.com/messaging/thread/2-abc123==/")
        assert result is True
        # thread_id extracted correctly and passed to find_by_intent
        intent_arg = analyzer.find_by_intent.call_args[0][1]
        assert "2-abc123==" in intent_arg

    def test_returns_false_when_sidebar_not_found(self):
        tab = self._make_nav_tab(sidebar_found=False)
        analyzer = _make_analyzer()
        nav = LinkedInNavigator(analyzer=analyzer)
        result = nav.open_thread(tab, "2-abc123==")
        assert result is False
        # should stop before calling analyzer
        analyzer.find_by_intent.assert_not_called()

    def test_returns_false_when_messages_not_found(self):
        tab = self._make_nav_tab(msgs_found=False)
        analyzer = _make_analyzer(find_by_intent=42)
        nav = LinkedInNavigator(analyzer=analyzer)
        result = nav.open_thread(tab, "2-abc123==")
        assert result is False

    def test_v3_bug_fix_no_direct_thread_url(self):
        """
        V3 bug: _watcher_loop navigated directly to /messaging/thread/<id>.
        V4 fix: open_thread() always goes to /messaging/ first, then clicks sidebar.
        """
        tab = self._make_nav_tab()
        analyzer = _make_analyzer(find_by_intent=42)
        nav = LinkedInNavigator(analyzer=analyzer)
        nav.open_thread(tab, "2-abc123==")

        navigated_url = tab.navigate.call_args[0][0]
        assert navigated_url == "https://www.linkedin.com/messaging/"
        assert "/thread/" not in navigated_url

    def test_bug1_sidebar_selector_fixed(self):
        """Bug 1: sidebar selector must be the conversations-list variant."""
        tab = self._make_nav_tab()
        analyzer = _make_analyzer(find_by_intent=1)
        nav = LinkedInNavigator(analyzer=analyzer)
        nav.open_thread(tab, "2-abc123==")
        sel = tab.wait_for_selector.call_args_list[0][0][0]
        assert sel == ".msg-conversations-container__conversations-list"
        assert sel != ".msg-conversations-container"

    def test_open_thread_uses_pageanalyzer_fallback(self):
        """
        Bug 2 fallback: when PageAnalyzer.find_by_intent returns None,
        the JS fallback (click first .msg-conversation-listitem__link) is called.
        """
        tab = self._make_nav_tab()
        analyzer = _make_analyzer(find_by_intent=None)  # LLM found nothing
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.open_thread(tab, "2-abc123==")

        # click_node must NOT have been called (no node_id)
        analyzer.click_node.assert_not_called()
        # JS fallback must have been called
        tab.js.assert_called_once()
        js_code = tab.js.call_args[0][0]
        assert ".msg-conversation-listitem__link" in js_code
        assert "el.click()" in js_code
        # Overall result depends on msgs_found (True by default)
        assert result is True


# ---------------------------------------------------------------------------
# LinkedInNavigator.get_last_message()
# ---------------------------------------------------------------------------

class TestLinkedInNavigatorGetLastMessage:

    def test_returns_last_message_text(self):
        tab = _make_tab()
        analyzer = _make_analyzer(find_last_message=None)
        nav = LinkedInNavigator(analyzer=analyzer)
        with patch.object(tab, "wait_last", return_value="Hey, how are you?") as mock_wl:
            result = nav.get_last_message(tab)
        assert result == "Hey, how are you?"
        sel = mock_wl.call_args[0][0]
        assert "msg-s-event-listitem__body" in sel

    def test_returns_none_when_no_messages(self):
        tab = _make_tab()
        analyzer = _make_analyzer(find_last_message=None)
        nav = LinkedInNavigator(analyzer=analyzer)
        with patch.object(tab, "wait_last", return_value=None):
            result = nav.get_last_message(tab)
        assert result is None

    def test_get_last_message_uses_analyzer_first(self):
        """
        When PageAnalyzer.find_last_message() returns a value,
        it is returned directly and wait_last() is NOT called.
        """
        tab = _make_tab()
        analyzer = _make_analyzer(find_last_message="Dame dinero bro")
        nav = LinkedInNavigator(analyzer=analyzer)
        tab.wait_last = MagicMock()

        result = nav.get_last_message(tab)

        assert result == "Dame dinero bro"
        analyzer.find_last_message.assert_called_once_with(tab)
        tab.wait_last.assert_not_called()


# ---------------------------------------------------------------------------
# LinkedInNavigator.send_message()
# ---------------------------------------------------------------------------

class TestLinkedInNavigatorSendMessage:

    def test_send_message_uses_pageanalyzer_nodes(self):
        """
        Bug 3: send_message should use PageAnalyzer.find_input_box /
        find_send_button + type_in_node / click_node when nodes are found.
        CSS selectors must NOT be used as the primary path.
        """
        tab = _make_tab()
        tab.wait_for_selector = MagicMock()
        analyzer = _make_analyzer(find_input_box=10, find_send_button=20)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.send_message(tab, "Hello world!")

        assert result is True
        analyzer.find_input_box.assert_called_once_with(tab)
        analyzer.type_in_node.assert_called_once_with(tab, 10, "Hello world!")
        analyzer.find_send_button.assert_called_once_with(tab)
        analyzer.click_node.assert_called_once_with(tab, 20)
        # CSS wait_for_selector must NOT have been called (analyzer handled it)
        tab.wait_for_selector.assert_not_called()

    def test_send_message_css_fallback_when_analyzer_returns_none(self):
        """When PageAnalyzer returns None for both nodes, fallback to CSS."""
        tab = _make_tab()
        tab.wait_for_selector = MagicMock(return_value=True)
        tab.click = MagicMock(return_value=True)
        tab.send = MagicMock(return_value={})
        analyzer = _make_analyzer(find_input_box=None, find_send_button=None)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.send_message(tab, "Hello fallback!")

        assert result is True
        # wait_for_selector called for compose box
        wfs_sels = [c[0][0] for c in tab.wait_for_selector.call_args_list]
        assert any("msg-form__contenteditable" in s for s in wfs_sels)
        # Input.insertText called
        insert_calls = [c for c in tab.send.call_args_list
                        if c[0][0] == "Input.insertText"]
        assert insert_calls
        assert insert_calls[0][0][1]["text"] == "Hello fallback!"

    def test_returns_false_when_compose_box_not_found_in_css_fallback(self):
        tab = _make_tab()
        tab.wait_for_selector = MagicMock(return_value=False)
        tab.send = MagicMock(return_value={})
        analyzer = _make_analyzer(find_input_box=None, find_send_button=None)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.send_message(tab, "Hello")

        assert result is False
        insert_calls = [c for c in tab.send.call_args_list
                        if c[0][0] == "Input.insertText"]
        assert not insert_calls

    def test_returns_false_when_send_button_not_found(self):
        """CSS fallback: click returns False → send_message returns False."""
        tab = _make_tab()
        tab.wait_for_selector = MagicMock(return_value=True)
        tab.click = MagicMock(return_value=False)
        tab.send = MagicMock(return_value={})
        analyzer = _make_analyzer(find_input_box=None, find_send_button=None)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.send_message(tab, "Hello")

        assert result is False

    def test_send_message_happy_path_via_css(self):
        """Legacy happy path: both nodes None → CSS path works end-to-end."""
        tab = _make_tab()
        tab.wait_for_selector = MagicMock(return_value=True)
        tab.click = MagicMock(return_value=True)
        tab.send = MagicMock(return_value={})
        analyzer = _make_analyzer(find_input_box=None, find_send_button=None)
        nav = LinkedInNavigator(analyzer=analyzer)

        result = nav.send_message(tab, "Hello world!")

        assert result is True
        insert_call = [c for c in tab.send.call_args_list
                       if c[0][0] == "Input.insertText"]
        assert insert_call
        assert insert_call[0][0][1]["text"] == "Hello world!"
