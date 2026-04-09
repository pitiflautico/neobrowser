"""
tests/test_session_cookies.py

Unit tests for F06: Cookie Persistence to Disk.
All tests use tmp_path fixture — no real Chrome process or real disk writes.
"""
from __future__ import annotations

import json
import stat
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.session import Session, COOKIES_BASE, _validate_cookie_path


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab(cookies=None):
    tab = MagicMock()
    tab.get_cookies.return_value = cookies or [
        {"name": "li_at", "value": "token123", "domain": ".linkedin.com"},
        {"name": "JSESSIONID", "value": "abc", "domain": ".linkedin.com"},
    ]
    return tab


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestSessionCookiePersistence:

    def test_save_cookies_writes_json(self, tmp_path):
        """save_cookies() writes valid JSON containing all cookies from tab."""
        custom_base = tmp_path / "cookies"
        session = Session("test-profile")
        tab = _make_tab()
        cookie_path = custom_base / "test-profile.json"

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            session.save_cookies(tab, path=cookie_path)

        assert cookie_path.exists()
        written = json.loads(cookie_path.read_text(encoding="utf-8"))
        assert written == tab.get_cookies.return_value

    def test_save_cookies_permissions(self, tmp_path):
        """save_cookies() sets file permissions to 0600."""
        custom_base = tmp_path / "cookies"
        session = Session("test-profile")
        tab = _make_tab()
        cookie_path = custom_base / "test-profile.json"

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            session.save_cookies(tab, path=cookie_path)

        mode = stat.S_IMODE(cookie_path.stat().st_mode)
        assert mode == 0o600

    def test_restore_cookies_calls_set_cookies(self, tmp_path):
        """restore_cookies() calls tab.set_cookies with the cookies from disk."""
        custom_base = tmp_path / "cookies"
        custom_base.mkdir(parents=True)
        session = Session("test-profile")
        cookies = [
            {"name": "li_at", "value": "token123", "domain": ".linkedin.com"},
            {"name": "JSESSIONID", "value": "abc", "domain": ".linkedin.com"},
        ]
        cookie_path = custom_base / "test-profile.json"
        cookie_path.write_text(json.dumps(cookies), encoding="utf-8")
        cookie_path.chmod(0o600)

        tab = MagicMock()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            session.restore_cookies(tab, path=cookie_path)

        tab.set_cookies.assert_called_once_with(cookies)

    def test_restore_cookies_returns_count(self, tmp_path):
        """restore_cookies() returns the number of cookies restored."""
        custom_base = tmp_path / "cookies"
        custom_base.mkdir(parents=True)
        session = Session("test-profile")
        cookies = [
            {"name": "li_at", "value": "token123", "domain": ".linkedin.com"},
            {"name": "JSESSIONID", "value": "abc", "domain": ".linkedin.com"},
            {"name": "lang", "value": "en", "domain": ".linkedin.com"},
        ]
        cookie_path = custom_base / "test-profile.json"
        cookie_path.write_text(json.dumps(cookies), encoding="utf-8")

        tab = MagicMock()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            count = session.restore_cookies(tab, path=cookie_path)

        assert count == 3

    def test_restore_cookies_missing_file(self, tmp_path):
        """restore_cookies() returns 0 when file does not exist — no exception."""
        custom_base = tmp_path / "cookies"
        session = Session("test-profile")
        missing = custom_base / "test-profile.json"

        tab = MagicMock()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            count = session.restore_cookies(tab, path=missing)

        assert count == 0
        tab.set_cookies.assert_not_called()

    def test_restore_cookies_corrupt_json(self, tmp_path):
        """restore_cookies() returns 0 on corrupt JSON — no exception raised."""
        custom_base = tmp_path / "cookies"
        custom_base.mkdir(parents=True)
        session = Session("test-profile")
        cookie_path = custom_base / "test-profile.json"
        cookie_path.write_text("not json {{{{", encoding="utf-8")

        tab = MagicMock()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            count = session.restore_cookies(tab, path=cookie_path)

        assert count == 0
        tab.set_cookies.assert_not_called()

    def test_path_outside_cookies_base_raises(self, tmp_path):
        """save_cookies()/restore_cookies() raise ValueError for paths outside COOKIES_BASE."""
        custom_base = tmp_path / "cookies"
        session = Session("test-profile")
        evil_path = tmp_path / "evil.json"  # outside custom_base

        tab = _make_tab()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            with pytest.raises(ValueError, match="Cookie path must be under"):
                session.save_cookies(tab, path=evil_path)

    def test_default_path_uses_profile_name(self, tmp_path):
        """Default cookie path is COOKIES_BASE / {profile_name}.json."""
        custom_base = tmp_path / "cookies"
        session = Session("my-profile")
        tab = _make_tab()

        with patch("tools.v4.session.COOKIES_BASE", custom_base):
            session.save_cookies(tab)

        expected = custom_base / "my-profile.json"
        assert expected.exists()
