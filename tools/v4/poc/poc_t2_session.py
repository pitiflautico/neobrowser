"""
T2 PoC: Session — open_tab, cookie set/get, zombie recovery, context manager.

Requires Chrome NOT running (Session.ensure() launches it).
Usage: python3 poc_t2_session.py
"""
from __future__ import annotations

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', '..'))

from tools.v4.session import Session


def main() -> None:
    print("[T2 PoC] Starting Session test")

    # --- 1. Session launches Chrome automatically ---
    with Session("poc-t2") as session:
        print(f"[T2 PoC] Session created: {session}")

        chrome = session.ensure()
        print(f"[T2 PoC] Chrome port={chrome.port}, pid={chrome.pid}")
        print(f"[T2 PoC] health_check={chrome.health_check()}")

        # --- 2. Open tab and navigate ---
        with session.open_tab() as tab:
            print(f"[T2 PoC] Tab opened: id={tab._tab_id}")
            tab.navigate("https://example.com", wait_s=2.0)
            title = tab.js("return document.title")
            print(f"[T2 PoC] Page title: {title!r}")

            # --- 3. Cookie set/get round-trip ---
            tab.set_cookies([{
                "name": "test_cookie",
                "value": "hello_v4",
                "domain": "example.com",
                "path": "/",
            }])
            cookies = tab.get_cookies(url="https://example.com")
            names = [c["name"] for c in cookies]
            print(f"[T2 PoC] Cookies after set: {names}")
            assert "test_cookie" in names, f"Cookie not found! Got: {names}"
            print("[T2 PoC] Cookie round-trip: PASS")

        # --- 4. ensure() reuses healthy Chrome ---
        chrome2 = session.ensure()
        assert chrome2 is chrome, "ensure() should reuse healthy Chrome"
        print("[T2 PoC] ensure() reuse: PASS")

    # --- 5. Context manager closed Chrome ---
    assert session._chrome is None, "Session._chrome should be None after close"
    print("[T2 PoC] Context manager cleanup: PASS")

    # --- 6. Zombie recovery: inject dead chrome, ensure() relaunches ---
    print("[T2 PoC] Testing zombie recovery...")
    from unittest.mock import MagicMock
    session2 = Session("poc-t2-zombie")
    dead = MagicMock()
    dead.health_check.return_value = False
    session2._chrome = dead

    with session2:
        chrome3 = session2.ensure()
        dead.kill.assert_called_once_with(force=True)
        print(f"[T2 PoC] Zombie killed, fresh Chrome on port={chrome3.port}: PASS")

    print("\n[T2 PoC] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
