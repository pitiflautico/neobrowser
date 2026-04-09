"""
tools/v4/tests/test_page_analyzer.py

Unit tests for PageAnalyzer (T3.5).
All tests use mock tab.send() — no real Chrome needed.
"""
from __future__ import annotations

import json
from unittest.mock import MagicMock, patch, call

import pytest

from tools.v4.page_analyzer import PageAnalyzer


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab(send_side_effect=None):
    """Return a mock ChromeTab with a configurable send() side effect."""
    tab = MagicMock()
    if send_side_effect is not None:
        tab.send.side_effect = send_side_effect
    return tab


def _ax_node(role: str, name: str, node_id: int, backend_id: int,
             ignored: bool = False) -> dict:
    """Build a raw CDP AX node dict in the format Accessibility.getFullAXTree returns."""
    return {
        "nodeId": str(node_id),
        "ignored": ignored,
        "role": {"value": role},
        "name": {"value": name},
        "backendDOMNodeId": backend_id,
    }


def _ax_response(nodes: list[dict]) -> dict:
    """Wrap a node list in the CDP response envelope."""
    return {"nodes": nodes}


# ---------------------------------------------------------------------------
# Test 1: snapshot() filters correctly
# ---------------------------------------------------------------------------

class TestSnapshot:
    def test_filters_generic_and_ignored_nodes(self):
        """generic + ignored nodes are excluded; valid button is kept."""
        nodes = [
            _ax_node("generic", "wrapper", 1, 101),
            _ax_node("button", "Send", 2, 102),
            _ax_node("none", "", 3, 103),
            _ax_node("presentation", "logo", 4, 104),
            _ax_node("listitem", "Hello", 5, 105),
            _ax_node("button", "ignored btn", 6, 106, ignored=True),
        ]
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        result = PageAnalyzer().snapshot(tab)
        roles = [n["role"] for n in result]
        assert "generic" not in roles
        assert "none" not in roles
        assert "presentation" not in roles
        # ignored button must not appear
        names = [n["name"] for n in result]
        assert "ignored btn" not in names
        # valid nodes are present
        assert "button" in roles
        assert "listitem" in roles

    def test_caps_at_150_nodes(self):
        """snapshot() returns at most 150 nodes."""
        # 200 buttons, all valid
        nodes = [_ax_node("button", f"btn{i}", i, 1000 + i) for i in range(200)]
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        result = PageAnalyzer().snapshot(tab)
        assert len(result) <= 150

    def test_interactive_nodes_prioritised_over_text_nodes(self):
        """When cap is hit, interactive roles appear before text roles."""
        # 100 listitems + 100 buttons — total 200, cap=150 means all 100 buttons + 50 listitems
        listitems = [_ax_node("listitem", f"msg{i}", i, 1000 + i) for i in range(100)]
        buttons = [_ax_node("button", f"btn{i}", 200 + i, 2000 + i) for i in range(100)]
        nodes = listitems + buttons
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        result = PageAnalyzer().snapshot(tab)
        assert len(result) == 150
        # First 100 entries should all be buttons (interactive first)
        assert all(n["role"] == "button" for n in result[:100])

    def test_node_without_backend_id_is_excluded(self):
        """Nodes with no backendDOMNodeId are silently dropped."""
        nodes = [
            {"nodeId": "1", "ignored": False, "role": {"value": "button"},
             "name": {"value": "Send"}},  # no backendDOMNodeId
            _ax_node("button", "Submit", 2, 202),
        ]
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        result = PageAnalyzer().snapshot(tab)
        assert len(result) == 1
        assert result[0]["backendNodeId"] == 202

    def test_text_node_without_name_is_excluded(self):
        """listitem with empty name is excluded; button with empty name is kept."""
        nodes = [
            _ax_node("listitem", "", 1, 101),   # no name → excluded
            _ax_node("button", "", 2, 102),      # interactive → kept even without name
        ]
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        result = PageAnalyzer().snapshot(tab)
        assert len(result) == 1
        assert result[0]["role"] == "button"


# ---------------------------------------------------------------------------
# Test 2 & 3: find_send_button()
# ---------------------------------------------------------------------------

class TestFindSendButton:
    def _snap_tab(self, nodes):
        return _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))

    def test_finds_english_send(self):
        nodes = [_ax_node("button", "Send", 1, 101)]
        result = PageAnalyzer().find_send_button(self._snap_tab(nodes))
        assert result == 101

    def test_finds_spanish_enviar(self):
        nodes = [_ax_node("button", "Enviar", 1, 101)]
        result = PageAnalyzer().find_send_button(self._snap_tab(nodes))
        assert result == 101

    def test_returns_none_when_no_match(self):
        nodes = [_ax_node("button", "Cancel", 1, 101)]
        result = PageAnalyzer().find_send_button(self._snap_tab(nodes))
        assert result is None

    def test_ignores_non_button_roles(self):
        nodes = [_ax_node("link", "Send", 1, 101)]
        result = PageAnalyzer().find_send_button(self._snap_tab(nodes))
        assert result is None


# ---------------------------------------------------------------------------
# Test 4: find_input_box()
# ---------------------------------------------------------------------------

class TestFindInputBox:
    def _snap_tab(self, nodes):
        return _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))

    def test_finds_textbox(self):
        nodes = [_ax_node("textbox", "Write a message", 1, 201)]
        result = PageAnalyzer().find_input_box(self._snap_tab(nodes))
        assert result == 201

    def test_finds_combobox(self):
        nodes = [_ax_node("combobox", "Search", 1, 202)]
        result = PageAnalyzer().find_input_box(self._snap_tab(nodes))
        assert result == 202

    def test_returns_none_when_no_input(self):
        nodes = [_ax_node("button", "Send", 1, 101)]
        result = PageAnalyzer().find_input_box(self._snap_tab(nodes))
        assert result is None


# ---------------------------------------------------------------------------
# Test 5 & 6: find_last_message()
# ---------------------------------------------------------------------------

class TestFindLastMessage:
    def _snap_tab(self, nodes):
        return _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))

    def test_returns_last_listitem(self):
        nodes = [
            _ax_node("listitem", "Hello there", 1, 301),
            _ax_node("listitem", "How are you?", 2, 302),
            _ax_node("listitem", "I am fine!", 3, 303),
        ]
        result = PageAnalyzer().find_last_message(self._snap_tab(nodes))
        assert result == "I am fine!"

    def test_returns_none_when_no_listitems(self):
        nodes = [_ax_node("button", "Send", 1, 101)]
        result = PageAnalyzer().find_last_message(self._snap_tab(nodes))
        assert result is None

    def test_returns_last_article_when_mixed(self):
        nodes = [
            _ax_node("listitem", "First msg", 1, 301),
            _ax_node("article", "Last msg via article", 2, 302),
        ]
        result = PageAnalyzer().find_last_message(self._snap_tab(nodes))
        assert result == "Last msg via article"

    def test_skips_empty_named_listitems(self):
        nodes = [
            _ax_node("listitem", "Real message", 1, 301),
            _ax_node("listitem", "", 2, 302),  # empty name → not a candidate
        ]
        result = PageAnalyzer().find_last_message(self._snap_tab(nodes))
        assert result == "Real message"


# ---------------------------------------------------------------------------
# Test 7: click_node()
# ---------------------------------------------------------------------------

class TestClickNode:
    def test_calls_dom_resolve_and_call_function(self):
        tab = MagicMock()
        tab.send.side_effect = [
            {"object": {"objectId": "obj-1"}},  # DOM.resolveNode
            {"result": {"value": True}},         # Runtime.callFunctionOn
        ]
        result = PageAnalyzer().click_node(tab, backend_node_id=999)
        assert result is True
        # First call: DOM.resolveNode
        first_call = tab.send.call_args_list[0]
        assert first_call[0][0] == "DOM.resolveNode"
        assert first_call[0][1]["backendNodeId"] == 999
        # Second call: Runtime.callFunctionOn with correct objectId
        second_call = tab.send.call_args_list[1]
        assert second_call[0][0] == "Runtime.callFunctionOn"
        assert second_call[0][1]["objectId"] == "obj-1"
        assert "this.click()" in second_call[0][1]["functionDeclaration"]

    def test_returns_false_on_error(self):
        tab = MagicMock()
        tab.send.side_effect = RuntimeError("CDP error")
        result = PageAnalyzer().click_node(tab, backend_node_id=999)
        assert result is False


# ---------------------------------------------------------------------------
# Test 8: type_in_node()
# ---------------------------------------------------------------------------

class TestTypeInNode:
    def test_calls_focus_then_insert_text(self):
        tab = MagicMock()
        tab.send.side_effect = [
            {"object": {"objectId": "obj-2"}},  # DOM.resolveNode
            {"result": {"value": True}},         # Runtime.callFunctionOn (focus)
            {},                                   # Input.insertText
        ]
        result = PageAnalyzer().type_in_node(tab, backend_node_id=888, text="Hello")
        assert result is True

        calls = tab.send.call_args_list
        assert calls[0][0][0] == "DOM.resolveNode"
        assert calls[0][0][1]["backendNodeId"] == 888

        assert calls[1][0][0] == "Runtime.callFunctionOn"
        assert calls[1][0][1]["objectId"] == "obj-2"
        assert "this.focus()" in calls[1][0][1]["functionDeclaration"]

        assert calls[2][0][0] == "Input.insertText"
        assert calls[2][0][1]["text"] == "Hello"

    def test_returns_false_on_error(self):
        tab = MagicMock()
        tab.send.side_effect = RuntimeError("CDP error")
        result = PageAnalyzer().type_in_node(tab, backend_node_id=888, text="Hi")
        assert result is False


# ---------------------------------------------------------------------------
# Test 9: find_by_intent() — mocked Anthropic client
# ---------------------------------------------------------------------------

class TestFindByIntent:
    def _nodes_for_intent(self):
        return [
            _ax_node("button", "Send message", 1, 501),
            _ax_node("textbox", "Write here", 2, 502),
        ]

    def test_llm_called_with_snapshot_and_intent(self):
        nodes = self._nodes_for_intent()
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        analyzer = PageAnalyzer()

        mock_message = MagicMock()
        mock_message.content = [MagicMock(text='{"backendNodeId": 501, "reason": "send button"}')]

        with patch("tools.v4.page_analyzer.PageAnalyzer._ask_llm") as mock_ask:
            mock_ask.return_value = {"backendNodeId": 501, "reason": "send button"}
            result = analyzer.find_by_intent(tab, intent="click the send button")

        assert result == 501
        mock_ask.assert_called_once()
        # Verify intent was passed to _ask_llm
        call_args = mock_ask.call_args
        assert "click the send button" in call_args[0][1]  # intent is 2nd positional arg

    def test_prompt_contains_snapshot_lines(self):
        """_ask_llm receives a snapshot_text that includes role|name|backendNodeId lines."""
        nodes = [_ax_node("button", "Send", 1, 601)]
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        analyzer = PageAnalyzer()

        captured = {}

        def fake_ask_llm(snapshot_text, intent, **kwargs):
            captured["snapshot_text"] = snapshot_text
            captured["intent"] = intent
            return {"backendNodeId": 601, "reason": "found"}

        analyzer._ask_llm = fake_ask_llm
        result = analyzer.find_by_intent(tab, intent="find the button")

        assert result == 601
        assert "button" in captured["snapshot_text"]
        assert "601" in captured["snapshot_text"]
        assert captured["intent"] == "find the button"

    def test_returns_none_on_llm_error(self):
        """find_by_intent returns None when _ask_llm returns None (never raises)."""
        nodes = self._nodes_for_intent()
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        analyzer = PageAnalyzer()
        analyzer._ask_llm = lambda *a, **kw: None

        result = analyzer.find_by_intent(tab, intent="some broken intent")
        assert result is None

    def test_returns_none_on_null_backend_node_id(self):
        """find_by_intent returns None when LLM says backendNodeId=null."""
        nodes = self._nodes_for_intent()
        tab = _make_tab(send_side_effect=lambda m, p=None: _ax_response(nodes))
        analyzer = PageAnalyzer()
        analyzer._ask_llm = lambda *a, **kw: {"backendNodeId": None, "reason": "not found"}

        result = analyzer.find_by_intent(tab, intent="something not on page")
        assert result is None
