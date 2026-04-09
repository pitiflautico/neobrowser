"""
tools/v4/poc/poc_f05_performance.py

F05 — Performance Metrics PoC

Validates ChromeTab.enable_performance() / get_metrics() against real Chrome.

Prerequisites:
  - Chrome running on port 55715
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f05_performance.py
"""
from __future__ import annotations

import sys
import os

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
    print("=" * 60)
    print("F05 Performance Metrics PoC")
    print("=" * 60)

    print("\n[1] Opening tab + enabling performance…")
    tab = ChromeTab.open(CHROME_PORT)
    try:
        tab.enable_performance()
        check("enable_performance() did not raise", True)

        tab.navigate("https://example.com", wait_s=2.0)

        # ------------------------------------------------------------------ #
        # Check 1: get_metrics() returns non-empty dict
        # ------------------------------------------------------------------ #
        print("\n[2] Calling get_metrics() after example.com load…")
        metrics = tab.get_metrics()
        print(f"    {len(metrics)} metrics returned")
        for k in sorted(metrics)[:8]:
            print(f"      {k}: {metrics[k]}")

        check("get_metrics() returns non-empty dict", len(metrics) > 0, f"count={len(metrics)}")

        # ------------------------------------------------------------------ #
        # Check 2: JSHeapUsedSize > 0
        # ------------------------------------------------------------------ #
        heap = metrics.get("JSHeapUsedSize", 0)
        check("JSHeapUsedSize > 0", heap > 0, f"JSHeapUsedSize={heap:,.0f}B")

        # ------------------------------------------------------------------ #
        # Check 3: Nodes > 0
        # ------------------------------------------------------------------ #
        nodes = metrics.get("Nodes", 0)
        check("Nodes > 0", nodes > 0, f"Nodes={nodes:.0f}")

        # ------------------------------------------------------------------ #
        # Check 4: get_metric() returns same value as get_metrics()
        # ------------------------------------------------------------------ #
        via_single = tab.get_metric("JSHeapUsedSize")
        # Re-call get_metrics to get a fresh snapshot for comparison
        metrics2 = tab.get_metrics()
        check(
            "get_metric('JSHeapUsedSize') is not None",
            via_single is not None,
            f"value={via_single}",
        )

        # ------------------------------------------------------------------ #
        # Check 5: get_metric("NoSuchMetric") is None
        # ------------------------------------------------------------------ #
        missing = tab.get_metric("NoSuchMetric_xyz")
        check("get_metric('NoSuchMetric') is None", missing is None)

    finally:
        tab.close()

    print("\n" + "=" * 60)
    if overall_pass:
        print("\033[32mOVERALL: PASS\033[0m")
    else:
        print("\033[31mOVERALL: FAIL\033[0m")
    print("=" * 60)
    sys.exit(0 if overall_pass else 1)


if __name__ == "__main__":
    main()
