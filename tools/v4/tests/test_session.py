"""
tests/test_session.py

Unit tests for T2: Session and ChromeTab cookie methods.

All tests are fully mocked — no Chrome process is started.
"""
from __future__ import annotations

import threading
from pathlib import Path
from unittest.mock import MagicMock, patch, call

import pytest

from tools.v4.session import Session, _validate_profile_name, CHROME_READY_TIMEOUT
from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_healthy_chrome(port: int = 9222, pid: int = 1234) -> MagicMock:
    """Mock ChromeProcess that reports healthy."""
    chrome = MagicMock()
    chrome.port = port
    chrome.pid = pid
    chrome.health_check.return_value = True
    chrome.is_alive.return_value = True
    chrome.port_alive.return_value = True
    return chrome


def _make_dead_chrome(port: int = 9222, pid: int = 1234) -> MagicMock:
    """Mock ChromeProcess that reports unhealthy."""
    chrome = MagicMock()
    chrome.port = port
    chrome.pid = pid
    chrome.health_check.return_value = False
    chrome.is_alive.return_value = False
    chrome.port_alive.return_value = False
    return chrome


def _make_tab(tab_id: str = "abc", port: int = 9222) -> ChromeTab:
    ws = MagicMock()
    ws.recv = MagicMock(return_value='{"id":1,"result":{}}')
    return ChromeTab(ws=ws, tab_id=tab_id, port=port)


# ---------------------------------------------------------------------------
# _validate_profile_name
# ---------------------------------------------------------------------------

class TestValidateProfileName:

    def test_accepts_simple_names(self):
        for name in ("linkedin", "default", "user123", "my-profile", "my_profile"):
            _validate_profile_name(name)  # should not raise

    def test_rejects_dotdot(self):
        with pytest.raises(ValueError):
            _validate_profile_name("../evil")

    def test_rejects_slash(self):
        with pytest.raises(ValueError):
            _validate_profile_name("foo/bar")

    def test_rejects_empty(self):
        with pytest.raises(ValueError):
            _validate_profile_name("")

    def test_rejects_space(self):
        with pytest.raises(ValueError):
            _validate_profile_name("my profile")

    def test_rejects_starts_with_dash(self):
        with pytest.raises(ValueError):
            _validate_profile_name("-evil")

    def test_rejects_too_long(self):
        with pytest.raises(ValueError):
            _validate_profile_name("a" * 65)

    def test_accepts_max_length(self):
        _validate_profile_name("a" * 64)  # exactly 64 — allowed


# ---------------------------------------------------------------------------
# Session.ensure()
# ---------------------------------------------------------------------------

class TestSessionEnsure:

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_launches_chrome_when_none(self, mock_launch, mock_wait):
        """ensure() launches Chrome when _chrome is None."""
        chrome = _make_healthy_chrome()
        mock_launch.return_value = chrome

        session = Session("test")
        result = session.ensure()

        mock_launch.assert_called_once()
        assert result is chrome
        assert session._chrome is chrome

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_reuses_healthy_chrome(self, mock_launch, mock_wait):
        """ensure() does NOT relaunch if chrome is already healthy."""
        chrome = _make_healthy_chrome()
        mock_launch.return_value = chrome

        session = Session("test")
        session.ensure()   # first call — launches
        session.ensure()   # second call — should reuse

        mock_launch.assert_called_once()  # only launched once

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_relaunches_on_dead_chrome(self, mock_launch, mock_wait):
        """ensure() kills zombie and launches fresh Chrome when health_check fails."""
        dead_chrome = _make_dead_chrome()
        fresh_chrome = _make_healthy_chrome(port=9999)
        mock_launch.return_value = fresh_chrome

        session = Session("test")
        session._chrome = dead_chrome  # inject zombie directly

        result = session.ensure()

        dead_chrome.kill.assert_called_once_with(force=True)  # zombie killed
        mock_launch.assert_called_once()                       # fresh chrome started
        assert result is fresh_chrome
        assert session._chrome is fresh_chrome

    @patch("tools.v4.session.wait_for_chrome", return_value=False)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_raises_when_chrome_not_ready(self, mock_launch, mock_wait):
        """ensure() raises RuntimeError if Chrome doesn't become ready in time."""
        chrome = _make_healthy_chrome()
        mock_launch.return_value = chrome

        session = Session("test")
        with pytest.raises(RuntimeError, match="did not become ready"):
            session.ensure()

        chrome.kill.assert_called_once_with(force=True)  # cleaned up on failure

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_profile_dir_uses_profile_name(self, mock_launch, mock_wait):
        """ensure() passes profile_dir = PROFILES_BASE / profile_name."""
        from tools.v4.session import PROFILES_BASE
        chrome = _make_healthy_chrome()
        mock_launch.return_value = chrome

        session = Session("myprofile")
        session.ensure()

        called_path = mock_launch.call_args[0][0]
        assert called_path == PROFILES_BASE / "myprofile"


# ---------------------------------------------------------------------------
# Security fixes
# ---------------------------------------------------------------------------

class TestSessionSecurityFixes:

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    def test_ensure_is_thread_safe(self, mock_launch, mock_wait):
        """TOCTOU fix: concurrent ensure() calls must launch Chrome only once."""
        import time as _time
        call_count = {"n": 0}

        def slow_launch(profile_dir):
            call_count["n"] += 1
            _time.sleep(0.05)  # simulate slow launch
            return _make_healthy_chrome(port=9000 + call_count["n"])

        mock_launch.side_effect = slow_launch
        session = Session("thread-safe")

        results = []
        errors = []

        def run():
            try:
                results.append(session.ensure())
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=run) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert not errors
        assert mock_launch.call_count == 1, (
            f"Chrome launched {mock_launch.call_count} times — TOCTOU not fixed"
        )

    def test_sibling_directory_bypass_blocked(self):
        """Security fix: profiles-sibling dir must be rejected by is_relative_to()."""
        from tools.v4.chrome_process import ChromeProcess, PROFILES_BASE
        from pathlib import Path

        # e.g. ~/.neorender/profiles-exfil/evil  — starts with PROFILES_BASE string
        # but is NOT inside PROFILES_BASE
        sibling = PROFILES_BASE.parent / (PROFILES_BASE.name + "-exfil") / "evil"

        with pytest.raises(ValueError, match="must be under"):
            ChromeProcess.launch(sibling)


# ---------------------------------------------------------------------------
# Session.open_tab()
# ---------------------------------------------------------------------------

class TestSessionOpenTab:

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    @patch("tools.v4.chrome_tab.ChromeTab.open")
    def test_open_tab_returns_chrome_tab(self, mock_tab_open, mock_launch, mock_wait):
        """open_tab() returns a ChromeTab instance."""
        chrome = _make_healthy_chrome(port=9222)
        mock_launch.return_value = chrome
        mock_tab_open.return_value = _make_tab()

        session = Session("test")
        tab = session.open_tab()

        mock_tab_open.assert_called_once_with(9222)
        assert isinstance(tab, ChromeTab)

    @patch("tools.v4.session.wait_for_chrome", return_value=True)
    @patch("tools.v4.session.ChromeProcess.launch")
    @patch("tools.v4.chrome_tab.ChromeTab.open")
    def test_open_tab_calls_ensure_first(self, mock_tab_open, mock_launch, mock_wait):
        """open_tab() calls ensure() to guarantee Chrome is healthy."""
        dead = _make_dead_chrome()
        fresh = _make_healthy_chrome(port=8888)
        mock_launch.return_value = fresh
        mock_tab_open.return_value = _make_tab(port=8888)

        session = Session("test")
        session._chrome = dead  # zombie

        session.open_tab()

        dead.kill.assert_called_once_with(force=True)  # zombie killed via ensure()
        mock_tab_open.assert_called_once_with(8888)    # tab opened on fresh port


# ---------------------------------------------------------------------------
# Session.close()
# ---------------------------------------------------------------------------

class TestSessionClose:

    def test_close_kills_chrome(self):
        """close() kills the Chrome process."""
        session = Session("test")
        chrome = _make_healthy_chrome()
        session._chrome = chrome

        session.close()

        chrome.kill.assert_called_once_with(force=True)
        assert session._chrome is None

    def test_close_is_idempotent(self):
        """close() is safe to call multiple times."""
        session = Session("test")
        session.close()  # _chrome is None — should not raise
        session.close()

    def test_context_manager_calls_close(self):
        """Session used as context manager calls close() on exit."""
        with Session("test") as session:
            chrome = _make_healthy_chrome()
            session._chrome = chrome

        chrome.kill.assert_called_once_with(force=True)
        assert session._chrome is None


# ---------------------------------------------------------------------------
# ChromeTab.set_cookies / get_cookies
# ---------------------------------------------------------------------------

class TestChromeTabCookies:

    def test_set_cookies_calls_network_setCookies(self):
        """set_cookies() sends Network.setCookies with the provided list."""
        tab = _make_tab()
        cookies = [{"name": "session", "value": "abc123", "domain": ".example.com"}]

        with patch.object(tab, "send", return_value={}) as mock_send:
            tab.set_cookies(cookies)

        mock_send.assert_called_once_with("Network.setCookies", {"cookies": cookies})

    def test_get_cookies_calls_network_getCookies(self):
        """get_cookies() sends Network.getCookies and returns the cookie list."""
        tab = _make_tab()
        expected = [{"name": "token", "value": "xyz", "domain": ".example.com"}]

        with patch.object(tab, "send", return_value={"cookies": expected}) as mock_send:
            result = tab.get_cookies()

        mock_send.assert_called_once_with("Network.getCookies", {})
        assert result == expected

    def test_get_cookies_with_url_filter(self):
        """get_cookies(url=...) passes urls=[url] to CDP."""
        tab = _make_tab()

        with patch.object(tab, "send", return_value={"cookies": []}) as mock_send:
            tab.get_cookies(url="https://example.com")

        mock_send.assert_called_once_with(
            "Network.getCookies", {"urls": ["https://example.com"]}
        )

    def test_get_cookies_returns_empty_on_missing_key(self):
        """get_cookies() returns [] if CDP response has no 'cookies' key."""
        tab = _make_tab()

        with patch.object(tab, "send", return_value={}):
            result = tab.get_cookies()

        assert result == []

    def test_set_cookies_v3_fix_any_time(self):
        """
        V3 bug: cookies synced once at startup.
        V4 fix: set_cookies() can be called after navigation — test that
        two sequential calls both go through without errors.
        """
        tab = _make_tab()
        batch1 = [{"name": "a", "value": "1", "domain": ".x.com"}]
        batch2 = [{"name": "b", "value": "2", "domain": ".x.com"}]

        with patch.object(tab, "send", return_value={}) as mock_send:
            tab.set_cookies(batch1)
            tab.set_cookies(batch2)

        assert mock_send.call_count == 2
