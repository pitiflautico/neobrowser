"""
T3 PoC: click(), wait_for_selector(), LinkedInNavigator flow.

Tests click() and wait_for_selector() against example.com (no auth needed).
LinkedIn thread opening requires a real authenticated session — documented
but not run headlessly here (use poc_t3_linkedin_live.py with cookies).

Usage: python3 poc_t3_navigation.py [port]
  port defaults to the V3 port file (~/.neorender/neo-browser-port.txt)
"""
from __future__ import annotations

import sys
import os
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', '..'))

from tools.v4.chrome_tab import ChromeTab
from tools.v4.linkedin_nav import LinkedInNavigator, _thread_id_from_url


def _get_port() -> int:
    if len(sys.argv) > 1:
        return int(sys.argv[1])
    port_file = Path.home() / ".neorender" / "neo-browser-port.txt"
    if port_file.exists():
        return int(port_file.read_text().strip())
    raise RuntimeError("Pass port as argument or run neo-browser first")


def main() -> None:
    port = _get_port()
    print(f"[T3 PoC] Using Chrome on port {port}")

    with ChromeTab.open(port) as tab:
        print(f"[T3 PoC] Tab opened: {tab._tab_id}")

        # --- 1. navigate + wait_for_selector ---
        tab.navigate("https://example.com", wait_s=2.0)
        found = tab.wait_for_selector("h1", timeout_s=5.0)
        print(f"[T3 PoC] wait_for_selector('h1') → {found}")
        assert found, "h1 not found on example.com"

        not_found = tab.wait_for_selector(".does-not-exist", timeout_s=0.5)
        print(f"[T3 PoC] wait_for_selector('.does-not-exist') → {not_found}")
        assert not not_found, "Should be False for absent selector"

        # --- 2. click() ---
        clicked = tab.click("h1")
        print(f"[T3 PoC] click('h1') → {clicked}")
        assert clicked is True

        no_click = tab.click(".absent-button")
        print(f"[T3 PoC] click('.absent-button') → {no_click}")
        assert no_click is False

        print("[T3 PoC] click() + wait_for_selector(): PASS")

    # --- 3. _thread_id_from_url utility ---
    cases = [
        ("https://www.linkedin.com/messaging/thread/2-abc123==/", "2-abc123=="),
        ("2-abc123==", "2-abc123=="),
        ("/2-abc123==/", "2-abc123=="),
    ]
    for url, expected in cases:
        got = _thread_id_from_url(url)
        assert got == expected, f"Expected {expected!r}, got {got!r} for {url!r}"
    print("[T3 PoC] _thread_id_from_url: PASS")

    # --- 4. LinkedInNavigator.open_thread — document the flow ---
    print("\n[T3 PoC] LinkedInNavigator flow (requires auth — NOT run headlessly here):")
    print("  nav = LinkedInNavigator()")
    print("  opened = nav.open_thread(tab, '2-abc123==')")
    print("  # 1. tab.navigate('https://www.linkedin.com/messaging/', wait_s=2.0)")
    print("  # 2. tab.wait_for_selector('.msg-conversations-container')")
    print("  # 3. tab.click('a[href*=\"/messaging/thread/2-abc123==\"]')")
    print("  # 4. tab.wait_for_selector('.msg-s-event-listitem__body')")
    print("  last = nav.get_last_message(tab)")
    print("  nav.send_message(tab, reply_text)")
    print("\n  V3 BUG (fixed): chrome_go('/messaging/thread/2-abc123==') directly")
    print("  → SPA never renders thread in headless. Fixed by sidebar-click flow.")

    print("\n[T3 PoC] ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
