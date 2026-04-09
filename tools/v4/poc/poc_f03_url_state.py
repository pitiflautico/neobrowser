"""
poc_f03_url_state.py

F03: URL State & Navigation History — PoC against a live Chrome instance.

Usage:
    python3 tools/v4/poc/poc_f03_url_state.py [port]

If port is not given, reads ~/.neorender/neo-browser-port.txt.
"""
from __future__ import annotations

import sys
import time
from pathlib import Path


def _read_port() -> int:
    if len(sys.argv) > 1:
        return int(sys.argv[1])
    port_file = Path.home() / ".neorender" / "neo-browser-port.txt"
    if port_file.exists():
        return int(port_file.read_text().strip())
    raise RuntimeError("No port provided and ~/.neorender/neo-browser-port.txt not found")


def check(label: str, condition: bool) -> bool:
    status = "PASS" if condition else "FAIL"
    print(f"  [{status}] {label}")
    return condition


def main() -> None:
    port = _read_port()
    print(f"Using Chrome on port {port}")
    print()

    from tools.v4.chrome_tab import ChromeTab

    results = []

    with ChromeTab.open(port) as tab:
        # Give the listener a moment to start
        time.sleep(0.2)

        # Step 1: Before any navigation — current_url should be ""
        url_before = tab.current_url()
        results.append(check(
            f'current_url() == "" before navigation (got: {url_before!r})',
            url_before == ""
        ))

        # Step 2: Navigate to example.com
        print("\nNavigating to https://example.com ...")
        tab.navigate("https://example.com", wait_s=2.0)

        # Give listener time to process the frameNavigated event
        time.sleep(0.3)

        url_after = tab.current_url()
        results.append(check(
            f'current_url() == "https://example.com/" after navigation (got: {url_after!r})',
            url_after == "https://example.com/"
        ))

        title = tab.page_title()
        results.append(check(
            f'page_title() == "Example Domain" (got: {title!r})',
            title == "Example Domain"
        ))

        # Step 3: Navigate to IANA
        print("\nNavigating to https://www.iana.org/domains/reserved ...")
        tab.navigate("https://www.iana.org/domains/reserved", wait_s=2.0)
        time.sleep(0.3)

        history = tab.navigation_history()
        results.append(check(
            f"len(navigation_history()) == 2 (got: {len(history)}, history: {history})",
            len(history) == 2
        ))

        iana_url = "https://www.iana.org/domains/reserved"
        results.append(check(
            f'is_at("{iana_url}") == True (current: {tab.current_url()!r})',
            tab.is_at(iana_url)
        ))

        results.append(check(
            'is_at("https://example.com/") == False',
            not tab.is_at("https://example.com/")
        ))

    # Summary
    print()
    passed = sum(results)
    total = len(results)
    print(f"Results: {passed}/{total} checks passed")
    print()
    if passed == total:
        print("OVERALL: PASS")
    else:
        print("OVERALL: FAIL")
        sys.exit(1)


if __name__ == "__main__":
    main()
