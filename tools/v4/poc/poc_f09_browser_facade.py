"""
tools/v4/poc/poc_f09_browser_facade.py

F09 — Browser Facade PoC

Validates that Browser() provides a clean 10-line API over the full stack.

Prerequisites:
  - Chrome running on port 55715
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f09_browser_facade.py
"""
from __future__ import annotations

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.browser import Browser

CHROME_PORT = 55715
PASS = "\033[32m[PASS]\033[0m"
FAIL = "\033[31m[FAIL]\033[0m"
overall_pass = True


def check(label: str, condition: bool, detail: str = "") -> None:
    global overall_pass
    status = PASS if condition else FAIL
    suffix = f"  ({detail})" if detail else ""
    print(f"  {status} {label}{suffix}")
    if not condition:
        overall_pass = False


def main() -> None:
    print("=" * 60)
    print("F09 Browser Facade PoC")
    print("=" * 60)

    # ------------------------------------------------------------------ #
    # Check 1: Browser() constructs without error
    # ------------------------------------------------------------------ #
    print("\n[1] Constructing Browser(profile='default')…")
    b = Browser(profile="default", pool_size=2)
    check("Browser() constructed without error", True)
    print(f"    {b!r}")

    try:
        # ------------------------------------------------------------------ #
        # Check 2: open() returns a tab and navigates
        # ------------------------------------------------------------------ #
        print("\n[2] b.open('https://example.com')…")
        tab = b.open("https://example.com", wait_s=3.0)
        check("open() returns a tab", tab is not None)
        print(f"    tab={tab!r}")

        # ------------------------------------------------------------------ #
        # Check 3: screenshot() returns PNG bytes > 0
        # ------------------------------------------------------------------ #
        print("\n[3] b.screenshot(tab)…")
        png = b.screenshot(tab)
        PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
        check("screenshot() returns PNG bytes > 0", isinstance(png, bytes) and len(png) > 0, f"size={len(png):,}B")
        check("PNG magic header", png[:8] == PNG_MAGIC)

        # ------------------------------------------------------------------ #
        # Check 4: network_log() has >= 1 entry
        # ------------------------------------------------------------------ #
        print("\n[4] b.network_log(tab)…")
        # Need to enable network first (tab was acquired without it)
        tab.enable_network()
        # Navigate again so network events fire
        tab.navigate("https://example.com", wait_s=2.0)
        net = b.network_log(tab)
        check("network_log() has >= 1 entry", len(net) >= 1, f"count={len(net)}")

        # ------------------------------------------------------------------ #
        # Check 5: metrics() returns JSHeapUsedSize > 0
        # ------------------------------------------------------------------ #
        print("\n[5] b.metrics(tab)…")
        m = b.metrics(tab)
        heap = m.get("JSHeapUsedSize", 0)
        check("metrics() has JSHeapUsedSize > 0", heap > 0, f"JSHeapUsedSize={heap:,.0f}B")

        # ------------------------------------------------------------------ #
        # Check 6: close_tab() does not raise
        # ------------------------------------------------------------------ #
        print("\n[6] b.close_tab(tab)…")
        b.close_tab(tab)
        check("close_tab() did not raise", True)

    finally:
        b.close()

    # ------------------------------------------------------------------ #
    # Check 7: context manager — close called on exit
    # ------------------------------------------------------------------ #
    print("\n[7] Testing context manager with Browser()…")
    closed_cleanly = False
    try:
        with Browser(profile="default") as b2:
            tab2 = b2.open("https://example.com", wait_s=2.0)
            b2.close_tab(tab2)
        closed_cleanly = True
    except Exception as e:
        print(f"    Exception: {e}")
    check("context manager: Browser closed on exit", closed_cleanly)

    print("\n" + "=" * 60)
    if overall_pass:
        print("\033[32mOVERALL: PASS\033[0m")
    else:
        print("\033[31mOVERALL: FAIL\033[0m")
    print("=" * 60)
    sys.exit(0 if overall_pass else 1)


if __name__ == "__main__":
    main()
