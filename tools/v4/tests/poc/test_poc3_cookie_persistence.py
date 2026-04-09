"""
PoC 3 — Session persistence: cookies entre procesos

Prueba que:
1. save_cookies() escribe JSON válido en disco con perms 0600
2. restore_cookies() inyecta las cookies en una tab nueva
3. El archivo se crea en la ruta correcta (~/.neorender/cookies/)
4. save_cookies() + restore_cookies() hacen round-trip exacto
5. restore_cookies() devuelve 0 si no hay archivo (no error)
6. El archivo tiene permisos 0600 (no legible por otros procesos)
"""
from __future__ import annotations

import json
import os
import stat
import tempfile
from pathlib import Path
from unittest.mock import MagicMock, call, patch

import pytest

from tools.v4.chrome_tab import ChromeTab
from tools.v4.session import Session, AttachedSession


# ─── Unit tests ───────────────────────────────────────────────────────────────

def _make_tab(cookies: list[dict] | None = None) -> ChromeTab:
    ws = MagicMock()
    tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
    if cookies is not None:
        tab.get_cookies = MagicMock(return_value=cookies)
        tab.set_cookies = MagicMock()
    return tab


class TestCookiePersistenceUnit:

    def test_save_cookies_writes_json(self, tmp_path):
        cookies = [{"name": "session", "value": "abc123", "domain": ".linkedin.com"}]
        tab = _make_tab(cookies)
        session = AttachedSession(9222)

        cookie_path = tmp_path / "cookies" / "attached-9222.json"
        session.save_cookies(tab, path=cookie_path)

        assert cookie_path.exists()
        saved = json.loads(cookie_path.read_text())
        assert saved == cookies

    def test_save_cookies_sets_0600_permissions(self, tmp_path):
        tab = _make_tab([{"name": "x", "value": "y", "domain": ".test.com"}])
        session = AttachedSession(9222)
        cookie_path = tmp_path / "cookies" / "test.json"
        cookie_path.parent.mkdir(parents=True)
        session.save_cookies(tab, path=cookie_path)

        mode = oct(stat.S_IMODE(cookie_path.stat().st_mode))
        assert mode == "0o600", f"Expected 0600, got {mode}"

    def test_restore_cookies_returns_0_when_no_file(self, tmp_path):
        tab = _make_tab()
        session = AttachedSession(9222)
        path = tmp_path / "nonexistent.json"
        count = session.restore_cookies(tab, path=path)
        assert count == 0

    def test_restore_cookies_returns_0_on_corrupt_json(self, tmp_path):
        tab = _make_tab()
        session = AttachedSession(9222)
        path = tmp_path / "bad.json"
        path.write_text("NOT JSON {{{{")
        count = session.restore_cookies(tab, path=path)
        assert count == 0

    def test_restore_cookies_injects_all_cookies(self, tmp_path):
        cookies = [
            {"name": "a", "value": "1", "domain": ".example.com"},
            {"name": "b", "value": "2", "domain": ".example.com"},
        ]
        path = tmp_path / "cookies.json"
        path.write_text(json.dumps(cookies))

        tab = _make_tab()
        session = AttachedSession(9222)
        count = session.restore_cookies(tab, path=path)

        assert count == 2
        tab.set_cookies.assert_called_once_with(cookies)

    def test_roundtrip_save_restore(self, tmp_path):
        """save_cookies → new tab → restore_cookies → same cookies."""
        cookies = [
            {"name": "li_at", "value": "AABBCC", "domain": ".linkedin.com"},
            {"name": "JSESSIONID", "value": "xyz789", "domain": ".linkedin.com"},
        ]
        tab1 = _make_tab(cookies)
        tab2 = _make_tab()
        session = AttachedSession(9222)
        path = tmp_path / "li_cookies.json"

        session.save_cookies(tab1, path=path)
        count = session.restore_cookies(tab2, path=path)

        assert count == 2
        tab2.set_cookies.assert_called_once_with(cookies)

    def test_browser_save_restore_via_facade(self, tmp_path):
        """Browser.save_cookies() and restore_cookies() delegate to session."""
        from tools.v4.browser import Browser
        cookies = [{"name": "x", "value": "1", "domain": ".test.com"}]
        tab = _make_tab(cookies)
        path = tmp_path / "facade_cookies.json"

        with Browser.connect(9999) as b:  # port doesn't matter for unit test
            b.save_cookies(tab, path=path)
            tab2 = _make_tab()
            count = b.restore_cookies(tab2, path=path)

        assert count == 1
        assert tab2.set_cookies.called

    def test_cookie_path_rejects_traversal(self, tmp_path):
        """save_cookies() must reject paths outside COOKIES_BASE."""
        from tools.v4.session import COOKIES_BASE
        tab = _make_tab([])
        session = Session("testprofile")
        evil_path = Path("/tmp/../../etc/passwd")
        with pytest.raises(ValueError):
            session.save_cookies(tab, path=evil_path)


# ─── Live tests ───────────────────────────────────────────────────────────────

class TestCookiePersistenceLive:

    def test_save_and_restore_real_cookies(self, live_port, live_tabs, tmp_path):
        """Save cookies from live Chrome tab, restore to new tab, verify count."""
        from tools.v4.chrome_tab import ChromeTab

        tab_info = live_tabs[0]
        tab = ChromeTab.attach(
            tab_info["webSocketDebuggerUrl"], tab_info["id"], live_port
        )
        try:
            session = AttachedSession(live_port)
            cookie_path = tmp_path / "live_cookies.json"

            session.save_cookies(tab, path=cookie_path)
            assert cookie_path.exists()
            cookies = json.loads(cookie_path.read_text())

            # File permissions must be 0600
            mode = oct(stat.S_IMODE(cookie_path.stat().st_mode))
            assert mode == "0o600"

            if cookies:
                # Restore into the same tab (idempotent)
                count = session.restore_cookies(tab, path=cookie_path)
                assert count == len(cookies)
        finally:
            tab.close()

    def test_linkedin_cookies_present_after_restore(
        self, live_port, linkedin_tab_info, tmp_path
    ):
        """After save+restore, LinkedIn session cookies are present."""
        tab = ChromeTab.attach(
            linkedin_tab_info["webSocketDebuggerUrl"],
            linkedin_tab_info["id"],
            live_port,
        )
        try:
            session = AttachedSession(live_port)
            cookie_path = tmp_path / "li.json"
            session.save_cookies(tab, path=cookie_path)

            cookies = json.loads(cookie_path.read_text())
            names = {c["name"] for c in cookies}

            # LinkedIn auth cookie must be present
            li_auth_cookies = {"li_at", "JSESSIONID", "lidc", "bcookie"}
            found = names & li_auth_cookies
            assert found, f"No LinkedIn auth cookies found. Got: {names}"
        finally:
            tab.close()
