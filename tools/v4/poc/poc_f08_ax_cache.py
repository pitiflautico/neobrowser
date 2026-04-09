"""
tools/v4/poc/poc_f08_ax_cache.py

F08 — AX Snapshot Cache PoC

Validates that PageAnalyzer.snapshot() correctly caches AX tree results and
invalidates on navigation.

Prerequisites:
  - Chrome running on port 55715 (e.g. launched via neorender-v4 or directly)
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f08_ax_cache.py
"""
from __future__ import annotations

import sys
import time

# Ensure project root is on path when run directly
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.chrome_tab import ChromeTab
from tools.v4.page_analyzer import PageAnalyzer

CHROME_PORT = 55715
PASS = "\033[32m[PASS]\033[0m"
FAIL = "\033[31m[FAIL]\033[0m"
overall_pass = True


def check(label: str, condition: bool) -> None:
    global overall_pass
    status = PASS if condition else FAIL
    print(f"  {status} {label}")
    if not condition:
        overall_pass = False


def main() -> None:
    global overall_pass

    print("=" * 60)
    print("F08 AX Snapshot Cache PoC")
    print("=" * 60)

    # ------------------------------------------------------------------ #
    # Open tab
    # ------------------------------------------------------------------ #
    print("\n[1] Opening tab and navigating to LinkedIn messaging…")
    tab = ChromeTab.open(CHROME_PORT)
    try:
        tab.navigate("https://www.linkedin.com/messaging/", wait_s=3.0)
        print("    Navigated. Waiting 3s for sidebar to render…")
        time.sleep(3.0)

        analyzer = PageAnalyzer(cache_ttl_s=5.0)

        # ------------------------------------------------------------------ #
        # Check 1 & 2: cache hit, second call faster
        # ------------------------------------------------------------------ #
        print("\n[2] First snapshot call (cache miss — CDP fetch)…")
        t0 = time.monotonic()
        snap1 = analyzer.snapshot(tab)
        t1 = time.monotonic()
        ms1 = (t1 - t0) * 1000

        print("\n[3] Second snapshot call (should be cache hit)…")
        t2 = time.monotonic()
        snap2 = analyzer.snapshot(tab)
        t3 = time.monotonic()
        ms2 = (t3 - t2) * 1000

        print(f"\n    Timing: call1={ms1:.0f}ms  call2={ms2:.0f}ms")
        check("snap1 and snap2 have same length", len(snap1) == len(snap2))
        check(
            f"call 2 faster than call 1 (call2={ms2:.1f}ms < call1={ms1:.1f}ms)",
            ms2 < ms1,
        )
        check(
            "cache hit is under 5ms",
            ms2 < 5.0,
        )

        # ------------------------------------------------------------------ #
        # Check 3: navigation invalidates cache
        # ------------------------------------------------------------------ #
        print("\n[4] Navigating to https://example.com (should invalidate cache)…")
        tab.navigate("https://example.com", wait_s=2.0)

        snap3 = analyzer.snapshot(tab)
        check(
            "snap3 (example.com) different length from snap1 (LinkedIn messaging)",
            len(snap3) != len(snap1),
        )

        # ------------------------------------------------------------------ #
        # Check 4: force=True bypasses cache
        # ------------------------------------------------------------------ #
        print("\n[5] Testing force=True…")
        snap4_a = analyzer.snapshot(tab)                      # from cache
        snap4_b = analyzer.snapshot(tab, force=True)          # forced fetch
        check(
            "force=True fetches even when cached (same content)",
            snap4_a == snap4_b,
        )
        # Verify force=True by counting CDP calls (timing unreliable on small pages)
        fetch_count = {"n": 0}
        original_fetch = analyzer._fetch_snapshot
        def counting_fetch(t):
            fetch_count["n"] += 1
            return original_fetch(t)
        analyzer._fetch_snapshot = counting_fetch
        analyzer.snapshot(tab)           # cache hit — no fetch
        analyzer.snapshot(tab, force=True)  # forced — fetch
        check(
            f"force=True triggers real fetch (expected 1 fetch, got {fetch_count['n']})",
            fetch_count["n"] == 1,
        )
        analyzer._fetch_snapshot = original_fetch  # restore

        # ------------------------------------------------------------------ #
        # Check 5: ttl=0 never caches — verified by fetch count
        # ------------------------------------------------------------------ #
        print("\n[6] Testing cache_ttl_s=0 (no caching)…")
        analyzer2 = PageAnalyzer(cache_ttl_s=0)
        fetch_count2 = {"n": 0}
        original_fetch2 = analyzer2._fetch_snapshot
        def counting_fetch2(t):
            fetch_count2["n"] += 1
            return original_fetch2(t)
        analyzer2._fetch_snapshot = counting_fetch2
        analyzer2.snapshot(tab)
        analyzer2.snapshot(tab)
        analyzer2.snapshot(tab)
        print(f"    ttl=0: 3 calls → {fetch_count2['n']} fetches")
        check(
            f"ttl=0 fetches every call (expected 3, got {fetch_count2['n']})",
            fetch_count2["n"] == 3,
        )
        analyzer2._fetch_snapshot = original_fetch2

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
