"""
tools/v4/poc/poc_f02_network_trace.py

F02 — Network Trace PoC

Validates that ChromeTab.enable_network() / get_network_requests() correctly
captures CDP Network events from a real Chrome tab.

Prerequisites:
  - Chrome running on port 55715 (launched via neorender-v4 or directly)
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f02_network_trace.py
"""
from __future__ import annotations

import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.chrome_tab import ChromeTab

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
    global overall_pass

    print("=" * 60)
    print("F02 Network Trace PoC")
    print("=" * 60)

    # ------------------------------------------------------------------ #
    # Setup: open tab, enable network BEFORE navigating
    # ------------------------------------------------------------------ #
    print("\n[1] Opening tab + enabling network capture…")
    tab = ChromeTab.open(CHROME_PORT)
    try:
        tab.enable_network()
        check("enable_network() did not raise", True)
        check("_network_enabled is True", tab._network_enabled)

        # ------------------------------------------------------------------ #
        # Navigate to example.com
        # ------------------------------------------------------------------ #
        print("\n[2] Navigating to https://example.com …")
        tab.navigate("https://example.com", wait_s=3.0)

        requests = tab.get_network_requests()
        print(f"    Captured {len(requests)} network request(s)")
        for r in requests[:5]:
            print(f"      {r['method']} {r['url'][:80]} → {r['status']} ({r['duration_ms']:.1f}ms)" if r['duration_ms'] else
                  f"      {r['method']} {r['url'][:80]} → {r['status']}")

        # ------------------------------------------------------------------ #
        # Check 1: main request captured
        # ------------------------------------------------------------------ #
        main_req = tab.get_network_request("example.com")
        check(
            "get_network_request('example.com') found",
            main_req is not None,
            str(main_req.get("url", "")) if main_req else "not found",
        )

        # ------------------------------------------------------------------ #
        # Check 2: status = 200
        # ------------------------------------------------------------------ #
        if main_req:
            check(
                f"main request status=200",
                main_req.get("status") == 200,
                f"actual status={main_req.get('status')}",
            )

            # ------------------------------------------------------------------ #
            # Check 3: duration_ms > 0
            # ------------------------------------------------------------------ #
            dur = main_req.get("duration_ms")
            check(
                "duration_ms > 0",
                dur is not None and dur > 0,
                f"duration_ms={dur}",
            )

            # ------------------------------------------------------------------ #
            # Check 4: encoded_data_length > 0
            # ------------------------------------------------------------------ #
            size = main_req.get("encoded_data_length")
            check(
                "encoded_data_length > 0",
                size is not None and size > 0,
                f"encoded_data_length={size}",
            )

        # ------------------------------------------------------------------ #
        # Check 5: get_network_requests() returns list with at least 1 entry
        # ------------------------------------------------------------------ #
        check(
            "get_network_requests() has >= 1 entry",
            len(requests) >= 1,
            f"count={len(requests)}",
        )

        # ------------------------------------------------------------------ #
        # Check 6: clear_network_log() empties buffer
        # ------------------------------------------------------------------ #
        print("\n[3] Testing clear_network_log()…")
        tab.clear_network_log()
        after_clear = tab.get_network_requests()
        check(
            "after clear_network_log(), get_network_requests() returns []",
            len(after_clear) == 0,
            f"count={len(after_clear)}",
        )
        check(
            "_network_enabled still True after clear",
            tab._network_enabled,
        )

        # ------------------------------------------------------------------ #
        # Check 7: loadingFailed — navigate to invalid URL, error field set
        # ------------------------------------------------------------------ #
        print("\n[4] Navigating to invalid URL to trigger loadingFailed…")
        # Use a nonexistent hostname to trigger net::ERR_NAME_NOT_RESOLVED
        # Note: navigate() will fail softly (Chrome shows error page)
        try:
            tab.navigate("https://this-host-does-not-exist-neorender-test.invalid/", wait_s=5.0)
        except Exception:
            pass

        failed_reqs = [r for r in tab.get_network_requests() if r.get("error")]
        print(f"    Requests with error field: {len(failed_reqs)}")
        if failed_reqs:
            print(f"    error = {failed_reqs[0]['error']}")

        check(
            "loadingFailed event: at least 1 request has error field set",
            len(failed_reqs) >= 1,
            f"failed_count={len(failed_reqs)}",
        )

    finally:
        tab.close()

    # ------------------------------------------------------------------ #
    # Final verdict
    # ------------------------------------------------------------------ #
    print("\n" + "=" * 60)
    if overall_pass:
        print("\033[32mOVERALL: PASS\033[0m")
    else:
        print("\033[31mOVERALL: FAIL\033[0m")
    print("=" * 60)
    sys.exit(0 if overall_pass else 1)


if __name__ == "__main__":
    main()
