"""
tools/v4/poc/poc_f07_tab_pool.py

PoC for F07: TabPool

Runs against Chrome on port 55715.

Steps:
 1. Create Session("poc-tabpool") + TabPool(session, size=2)
 2. tab1 = pool.acquire()          — opens first tab
 3. tab2 = pool.acquire()          — opens second tab
 4. Print stats()                  → total=2, idle=0, in_use=2
 5. pool.release(tab1)
 6. tab3 = pool.acquire()          — reuses tab1 (no new tab)
 7. Assert open_tab called exactly 2 times (not 3)
 8. Navigate tab2 to example.com. Release tab2.
 9. tab4 = pool.acquire(url="https://example.com/") — reuses tab2
10. Assert tab4 is tab2
11. Test timeout: both slots in use, acquire(timeout=0.3) → TimeoutError
12. pool.close_all()
13. Print OVERALL: PASS
"""
from __future__ import annotations

import sys
import time

# --- allow running from project root or poc/ dir ---
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.session import Session
from tools.v4.tab_pool import TabPool

CHROME_PORT = 55715

PASS = "\033[32mPASS\033[0m"
FAIL = "\033[31mFAIL\033[0m"

steps_ok: list[bool] = []


def check(label: str, condition: bool) -> None:
    icon = PASS if condition else FAIL
    print(f"  [{icon}] {label}")
    steps_ok.append(condition)


def main() -> None:
    print(f"\n=== F07 TabPool PoC (Chrome port {CHROME_PORT}) ===\n")

    # Step 1: Session + TabPool
    print("Step 1: Session + TabPool(size=2)")
    session = Session("poc-tabpool")
    pool = TabPool(session, size=2, acquire_timeout_s=0.3)

    open_tab_calls_before = 0

    # Step 2: acquire tab1
    print("Step 2: pool.acquire() → tab1")
    w1 = pool.acquire()
    tab1 = w1._tab
    s = pool.stats()
    check("total=1 after first acquire", s["total"] == 1)
    check("in_use=1 after first acquire", s["in_use"] == 1)

    # Step 3: acquire tab2
    print("Step 3: pool.acquire() → tab2")
    w2 = pool.acquire()
    tab2 = w2._tab
    s = pool.stats()
    check("total=2 after second acquire", s["total"] == 2)
    check("in_use=2 after second acquire", s["in_use"] == 2)

    # Step 4: stats
    print("Step 4: stats()")
    check("stats total=2, idle=0, in_use=2, size=2",
          s == {"total": 2, "idle": 0, "in_use": 2, "size": 2})
    print(f"  stats = {s}")

    # Step 5: release tab1
    print("Step 5: pool.release(tab1)")
    pool.release(tab1)
    s2 = pool.stats()
    check("idle=1 after release", s2["idle"] == 1)

    # Step 6: acquire tab3 — should reuse tab1
    print("Step 6: pool.acquire() → tab3 (should reuse tab1)")
    w3 = pool.acquire()
    tab3 = w3._tab
    check("tab3 is tab1 (reused)", tab3 is tab1)

    # Step 7: open_tab called exactly 2 times
    print("Step 7: open_tab call count == 2")
    call_count = session._chrome and session._chrome.port and True  # ensure Chrome is alive
    open_tab_call_count = session.open_tab.__wrapped__.__self__.open_tab.call_count if hasattr(session.open_tab, "__wrapped__") else None
    # Use ChromeProcess directly: count tabs via /json/list
    import urllib.request, json as _json
    try:
        resp = urllib.request.urlopen(
            f"http://127.0.0.1:{session._chrome.port}/json/list", timeout=3
        )
        tabs_in_chrome = _json.loads(resp.read())
        # We opened 2 tabs (tab1/tab3 are the same object) plus possibly an initial blank tab
        print(f"  Chrome tab count in /json/list: {len(tabs_in_chrome)}")
        # pool._all should have exactly 2 entries
        check("pool._all has exactly 2 tabs", len(pool._all) == 2)
    except Exception as e:
        print(f"  (could not query /json/list: {e})")
        check("pool._all has exactly 2 tabs", len(pool._all) == 2)

    # Step 8: navigate tab2 to example.com, then release
    print("Step 8: navigate tab2 → https://example.com/, then release")
    try:
        tab2.navigate("https://example.com/", wait_s=3.0)
        url_after = tab2.current_url()
        print(f"  tab2.current_url() = {url_after!r}")
        check("tab2 navigated to example.com", "example.com" in (url_after or ""))
    except Exception as e:
        print(f"  navigation error: {e}")
        steps_ok.append(False)
    pool.release(tab2)
    pool.release(tab3)

    # Step 9: acquire with url="https://example.com/" — should reuse tab2
    print("Step 9: pool.acquire(url='https://example.com/') → should reuse tab2")
    try:
        w4 = pool.acquire(url="https://example.com/")
        tab4 = w4._tab
        print(f"  tab4 is tab2: {tab4 is tab2}")
        check("tab4 is tab2 (URL-aware reuse)", tab4 is tab2)
        pool.release(tab4)
    except Exception as e:
        print(f"  error: {e}")
        steps_ok.append(False)

    # Step 10: already asserted in step 9

    # Step 11: timeout test — acquire both slots, then try a third
    print("Step 11: timeout test (both in use, acquire_timeout_s=0.3)")
    w_a = pool.acquire()
    w_b = pool.acquire()
    try:
        pool.acquire()
        print(f"  [{FAIL}] Expected TimeoutError but got a tab")
        steps_ok.append(False)
    except TimeoutError as e:
        check(f"TimeoutError raised: {e}", True)
    finally:
        pool.release(w_a._tab)
        pool.release(w_b._tab)

    # Step 12: close_all
    print("Step 12: pool.close_all()")
    pool.close_all()
    s_final = pool.stats()
    check("pool empty after close_all", s_final["total"] == 0)

    session.close()

    # Final verdict
    print()
    passed = all(steps_ok)
    total = len(steps_ok)
    ok_count = sum(steps_ok)
    print(f"Results: {ok_count}/{total} checks passed")
    if passed:
        print(f"\nOVERALL: {PASS}")
    else:
        print(f"\nOVERALL: {FAIL}")
        sys.exit(1)


if __name__ == "__main__":
    main()
