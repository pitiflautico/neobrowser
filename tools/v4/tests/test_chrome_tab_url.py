"""
tools/v4/tests/test_chrome_tab_url.py

F03: URL State & Navigation History — unit tests for ChromeTab
"""
from __future__ import annotations

import json
import queue
import threading
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab(current_url=None, page_title=None, history=None) -> ChromeTab:
    """Create a ChromeTab with a mock WS, bypassing open()."""
    ws = MagicMock()
    ws.send = MagicMock()
    # recv will be controlled per test
    tab = ChromeTab(ws=ws, tab_id="test-tab-id", port=9222)
    if current_url is not None:
        tab._current_url = current_url
    if page_title is not None:
        tab._page_title = page_title
    if history is not None:
        tab._history = list(history)
    return tab


# ---------------------------------------------------------------------------
# 1. current_url — empty before navigation (JS fallback returns "")
# ---------------------------------------------------------------------------

def test_current_url_empty_before_navigation():
    tab = _make_tab()
    assert tab._current_url is None

    # JS fallback: window.location.href on a fresh tab returns "" or about:blank
    # We mock js() to return "" (empty/blank)
    with patch.object(tab, "js", return_value="") as mock_js:
        result = tab.current_url()
    assert result == ""
    mock_js.assert_called_once_with("return window.location.href")


# ---------------------------------------------------------------------------
# 2. current_url — returns cached value WITHOUT calling send()
# ---------------------------------------------------------------------------

def test_current_url_from_cache():
    tab = _make_tab(current_url="https://example.com/")
    with patch.object(tab, "send") as mock_send:
        with patch.object(tab, "js") as mock_js:
            result = tab.current_url()
    assert result == "https://example.com/"
    mock_send.assert_not_called()
    mock_js.assert_not_called()


# ---------------------------------------------------------------------------
# 3. current_url — JS fallback when _current_url is None
# ---------------------------------------------------------------------------

def test_current_url_js_fallback():
    tab = _make_tab()
    with patch.object(tab, "js", return_value="https://fallback.example.com/") as mock_js:
        result = tab.current_url()
    assert result == "https://fallback.example.com/"
    mock_js.assert_called_once_with("return window.location.href")
    # Should also be cached now
    assert tab._current_url == "https://fallback.example.com/"


# ---------------------------------------------------------------------------
# 4. page_title — fetches via JS when not cached
# ---------------------------------------------------------------------------

def test_page_title_from_js():
    tab = _make_tab()
    assert tab._page_title is None
    with patch.object(tab, "js", return_value="Example Domain") as mock_js:
        result = tab.page_title()
    assert result == "Example Domain"
    mock_js.assert_called_once_with("return document.title")
    assert tab._page_title == "Example Domain"


# ---------------------------------------------------------------------------
# 5. page_title — returns cached value without JS call
# ---------------------------------------------------------------------------

def test_page_title_cached():
    tab = _make_tab(page_title="Cached Title")
    with patch.object(tab, "js") as mock_js:
        result = tab.page_title()
    assert result == "Cached Title"
    mock_js.assert_not_called()


# ---------------------------------------------------------------------------
# 6. navigation_history — appends 3 frameNavigated events
# ---------------------------------------------------------------------------

def test_navigation_history_appends():
    tab = _make_tab()
    urls = [
        "https://example.com/",
        "https://example.com/page1",
        "https://example.com/page2",
    ]
    for url in urls:
        tab._handle_page_event({
            "method": "Page.frameNavigated",
            "params": {"frame": {"url": url}},
        })
    history = tab.navigation_history()
    assert history == urls


# ---------------------------------------------------------------------------
# 7. navigation_history — skips about:blank
# ---------------------------------------------------------------------------

def test_navigation_history_skips_about_blank():
    tab = _make_tab()
    tab._handle_page_event({
        "method": "Page.frameNavigated",
        "params": {"frame": {"url": "about:blank"}},
    })
    tab._handle_page_event({
        "method": "Page.frameNavigated",
        "params": {"frame": {"url": "https://real.example.com/"}},
    })
    history = tab.navigation_history()
    assert history == ["https://real.example.com/"]
    assert "about:blank" not in history


# ---------------------------------------------------------------------------
# 8. navigation_history — capped at 50, oldest dropped
# ---------------------------------------------------------------------------

def test_navigation_history_cap_50():
    tab = _make_tab()
    for i in range(51):
        tab._handle_page_event({
            "method": "Page.frameNavigated",
            "params": {"frame": {"url": f"https://example.com/page{i}"}},
        })
    history = tab.navigation_history()
    assert len(history) == 50
    # oldest (page0) should have been dropped
    assert "https://example.com/page0" not in history
    assert "https://example.com/page50" in history


# ---------------------------------------------------------------------------
# 9. is_at — True and False cases
# ---------------------------------------------------------------------------

def test_is_at_true_and_false():
    tab = _make_tab(current_url="https://example.com/")
    assert tab.is_at("https://example.com/") is True
    assert tab.is_at("https://other.com/") is False


# ---------------------------------------------------------------------------
# 10. event queue — send() routes non-matching messages to _event_queue
# ---------------------------------------------------------------------------

def test_event_queue_populated_by_send():
    """
    Simulate a send() call where ws.recv() returns an event message (no id)
    before returning the actual response. The event must land in _event_queue.
    """
    ws = MagicMock()
    tab = ChromeTab(ws=ws, tab_id="test", port=9222)

    event_msg = {"method": "Page.frameNavigated", "params": {"frame": {"url": "https://queued.example.com/"}}}
    response_msg = {"id": 1, "result": {}}

    call_count = 0

    def fake_recv(timeout=None):
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            return json.dumps(event_msg)
        return json.dumps(response_msg)

    ws.recv = fake_recv
    ws.send = MagicMock()

    tab.send("Runtime.evaluate", {"expression": "1"})

    assert not tab._event_queue.empty()
    queued = tab._event_queue.get_nowait()
    assert queued["method"] == "Page.frameNavigated"
