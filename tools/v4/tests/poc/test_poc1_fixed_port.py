"""
PoC 1 — Chrome con puerto fijo

Prueba que:
1. ChromeLauncher detecta Chrome ya corriendo (is_running)
2. La versión reportada es Chrome real
3. Port discovery encuentra el Chrome disponible
4. El mismo puerto devuelve las mismas tabs (estabilidad de sesión)
"""
from __future__ import annotations

import json
import urllib.request

import pytest

from tools.v4.chrome_launcher import ChromeLauncher


class TestChromeLauncherUnit:
    """Unit tests — no Chrome required."""

    def test_is_running_returns_false_when_no_chrome(self):
        launcher = ChromeLauncher(port=19999)  # unlikely to be in use
        assert launcher.is_running() is False

    def test_launcher_default_port_is_9222(self):
        launcher = ChromeLauncher()
        assert launcher.port == 9222

    def test_launcher_profile_dir_under_neorender(self):
        from pathlib import Path
        launcher = ChromeLauncher(port=9222, profile="test")
        assert ".neorender" in str(launcher._profile_dir)
        assert "test" in str(launcher._profile_dir)


class TestChromeLauncherLive:
    """Live tests — require a running Chrome (any port)."""

    def test_is_running_returns_true(self, live_port):
        launcher = ChromeLauncher(port=live_port)
        assert launcher.is_running() is True

    def test_version_returns_browser_info(self, live_port):
        launcher = ChromeLauncher(port=live_port)
        info = launcher.version()
        assert "Browser" in info
        assert "Chrome" in info["Browser"]

    def test_port_stability_same_tabs_on_retry(self, live_port, live_tabs):
        """Same port → same tabs on second query (session stable)."""
        resp2 = urllib.request.urlopen(
            f"http://127.0.0.1:{live_port}/json/list", timeout=2
        )
        tabs2 = json.loads(resp2.read())
        urls1 = {t["url"] for t in live_tabs}
        urls2 = {t["url"] for t in tabs2}
        # At least the tabs present in round 1 are still there
        assert urls1 == urls2, f"Tabs changed between queries: {urls1 ^ urls2}"

    def test_ensure_is_idempotent_when_already_running(self, live_port):
        """ensure() on already-running Chrome must not raise."""
        launcher = ChromeLauncher(port=live_port)
        launcher.ensure()  # should not raise, not launch a second Chrome
        assert launcher.is_running()

    def test_port_discovery_finds_chrome(self, live_port):
        """The conftest port discovery mechanism works correctly."""
        from tools.v4.tests.poc.conftest import _find_chrome_port
        found = _find_chrome_port()
        assert found is not None
        assert found == live_port
