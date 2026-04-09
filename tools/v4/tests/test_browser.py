"""
tools/v4/tests/test_browser.py

Unit tests for F09 — Browser Facade.
All tests use mocks — no Chrome required.
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock, patch, call

import pytest

from tools.v4.browser import Browser


# ---------------------------------------------------------------------------
# Helpers — build a Browser with mocked internals
# ---------------------------------------------------------------------------


def make_browser(profile: str = "default") -> tuple["Browser", MagicMock, MagicMock, MagicMock]:
    """
    Return (browser, mock_session, mock_pool, mock_analyzer).
    Patches the three constructors so no real Chrome is needed.
    """
    mock_session = MagicMock()
    mock_pool = MagicMock()
    mock_analyzer = MagicMock()

    mock_pool.stats.return_value = {"total": 0, "idle": 0, "in_use": 0}

    with (
        patch("tools.v4.browser.Session", return_value=mock_session),
        patch("tools.v4.browser.TabPool", return_value=mock_pool),
        patch("tools.v4.browser.PageAnalyzer", return_value=mock_analyzer),
    ):
        b = Browser(profile=profile, pool_size=2, ax_cache_ttl_s=3.0)

    return b, mock_session, mock_pool, mock_analyzer


def make_mock_tab() -> MagicMock:
    tab = MagicMock()
    tab._performance_enabled = False
    tab.screenshot.return_value = b"\x89PNG\r\n\x1a\n" + b"\x00" * 50
    tab.get_console_logs.return_value = [{"level": "log", "text": "hi"}]
    tab.get_network_requests.return_value = [{"url": "https://example.com", "status": 200}]
    tab.get_metrics.return_value = {"JSHeapUsedSize": 1_000_000.0, "Nodes": 10.0}
    return tab


# ---------------------------------------------------------------------------
# Test 1: Browser() constructs Session + TabPool + PageAnalyzer
# ---------------------------------------------------------------------------


def test_browser_constructs_components():
    with (
        patch("tools.v4.browser.Session") as MockSession,
        patch("tools.v4.browser.TabPool") as MockPool,
        patch("tools.v4.browser.PageAnalyzer") as MockAnalyzer,
    ):
        MockPool.return_value.stats.return_value = {"total": 0, "idle": 0, "in_use": 0}
        b = Browser(profile="myprofile", pool_size=4, ax_cache_ttl_s=10.0)

    MockSession.assert_called_once_with("myprofile")
    MockPool.assert_called_once()
    MockAnalyzer.assert_called_once_with(cache_ttl_s=10.0)


# ---------------------------------------------------------------------------
# Test 2: open() calls pool.acquire() and returns ChromeTab
# ---------------------------------------------------------------------------


def test_open_calls_pool_acquire():
    b, _, mock_pool, _ = make_browser()
    mock_tab = make_mock_tab()
    mock_pool.acquire.return_value = mock_tab

    result = b.open("https://example.com")

    mock_pool.acquire.assert_called_once_with(url="https://example.com")
    assert result is mock_tab


# ---------------------------------------------------------------------------
# Test 3: close_tab() calls pool.release()
# ---------------------------------------------------------------------------


def test_close_tab_calls_pool_release():
    b, _, mock_pool, _ = make_browser()
    mock_tab = make_mock_tab()

    b.close_tab(mock_tab)

    mock_pool.release.assert_called_once_with(mock_tab)


# ---------------------------------------------------------------------------
# Test 4: close() calls pool.close_all() and session.close()
# ---------------------------------------------------------------------------


def test_close_shuts_down_all():
    b, mock_session, mock_pool, _ = make_browser()

    b.close()

    mock_pool.close_all.assert_called_once()
    mock_session.close.assert_called_once()


# ---------------------------------------------------------------------------
# Test 5: context manager __exit__ calls close()
# ---------------------------------------------------------------------------


def test_context_manager_calls_close():
    with (
        patch("tools.v4.browser.Session"),
        patch("tools.v4.browser.TabPool") as MockPool,
        patch("tools.v4.browser.PageAnalyzer"),
    ):
        MockPool.return_value.stats.return_value = {"total": 0, "idle": 0, "in_use": 0}
        with Browser() as b:
            mock_pool = b._pool
            mock_session = b._session

    mock_pool.close_all.assert_called_once()
    mock_session.close.assert_called_once()


# ---------------------------------------------------------------------------
# Test 6: screenshot() delegates to tab.screenshot()
# ---------------------------------------------------------------------------


def test_screenshot_delegates():
    b, _, _, _ = make_browser()
    tab = make_mock_tab()

    result = b.screenshot(tab, format="jpeg", quality=70)

    tab.screenshot.assert_called_once_with(format="jpeg", quality=70)
    assert result == tab.screenshot.return_value


# ---------------------------------------------------------------------------
# Test 7: console_logs() delegates to tab.get_console_logs()
# ---------------------------------------------------------------------------


def test_console_logs_delegates():
    b, _, _, _ = make_browser()
    tab = make_mock_tab()

    result = b.console_logs(tab)

    tab.get_console_logs.assert_called_once()
    assert result == tab.get_console_logs.return_value


# ---------------------------------------------------------------------------
# Test 8: network_log() delegates to tab.get_network_requests()
# ---------------------------------------------------------------------------


def test_network_log_delegates():
    b, _, _, _ = make_browser()
    tab = make_mock_tab()

    result = b.network_log(tab)

    tab.get_network_requests.assert_called_once()
    assert result == tab.get_network_requests.return_value


# ---------------------------------------------------------------------------
# Test 9: metrics() enables performance if not enabled, then delegates
# ---------------------------------------------------------------------------


def test_metrics_enables_and_delegates():
    b, _, _, _ = make_browser()
    tab = make_mock_tab()
    tab._performance_enabled = False

    result = b.metrics(tab)

    tab.enable_performance.assert_called_once()
    tab.get_metrics.assert_called_once()
    assert result == tab.get_metrics.return_value


def test_metrics_skips_enable_if_already_enabled():
    b, _, _, _ = make_browser()
    tab = make_mock_tab()
    tab._performance_enabled = True

    b.metrics(tab)

    tab.enable_performance.assert_not_called()
    tab.get_metrics.assert_called_once()


# ---------------------------------------------------------------------------
# Test 10: save_cookies() delegates to session.save_cookies()
# ---------------------------------------------------------------------------


def test_save_cookies_delegates():
    b, mock_session, _, _ = make_browser()
    tab = make_mock_tab()

    b.save_cookies(tab)

    mock_session.save_cookies.assert_called_once_with(tab, path=None)


# ---------------------------------------------------------------------------
# Test 11: restore_cookies() delegates to session.restore_cookies()
# ---------------------------------------------------------------------------


def test_restore_cookies_delegates_and_returns_count():
    b, mock_session, _, _ = make_browser()
    tab = make_mock_tab()
    mock_session.restore_cookies.return_value = 42

    count = b.restore_cookies(tab)

    mock_session.restore_cookies.assert_called_once_with(tab, path=None)
    assert count == 42
