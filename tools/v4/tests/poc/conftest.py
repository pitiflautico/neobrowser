"""
Shared fixtures for PoC integration tests.

Tests marked with @pytest.mark.live require a real Chrome running
on NEOBROWSER_PORT (default 9222). They are skipped automatically
when no Chrome is available.

Unit PoC tests (mocked) run always.
"""
from __future__ import annotations

import json
import os
import urllib.request

import pytest

LIVE_PORT = int(os.environ.get("NEOBROWSER_PORT", "9222"))


def _chrome_available(port: int) -> bool:
    try:
        urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.0)
        return True
    except Exception:
        return False


def _find_chrome_port() -> int | None:
    """Scan known ports for a running Chrome."""
    for port in [9222, 49451, 65315, 50965]:
        if _chrome_available(port):
            return port
    return None


@pytest.fixture(scope="session")
def live_port():
    """Return port of a running Chrome, or skip if none found."""
    port = _find_chrome_port()
    if port is None:
        pytest.skip("No Chrome running — start with: ./tools/v4/chrome_launcher.sh")
    return port


@pytest.fixture(scope="session")
def live_tabs(live_port):
    """Return list of open tabs on live_port."""
    resp = urllib.request.urlopen(f"http://127.0.0.1:{live_port}/json/list", timeout=2)
    return json.loads(resp.read())


@pytest.fixture(scope="session")
def linkedin_tab_info(live_tabs):
    """Return tab info for LinkedIn messaging thread, or skip."""
    tab = next(
        (t for t in live_tabs if "linkedin.com/messaging" in t.get("url", "")),
        None,
    )
    if tab is None:
        pytest.skip("No LinkedIn messaging tab open")
    return tab
