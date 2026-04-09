"""
F06 PoC: Cookie Persistence to Disk

Connects to a Chrome instance already running on port 55715 (LinkedIn session).
No Chrome process is launched — uses an existing one.

Usage: python3 poc_f06_cookies.py
"""
from __future__ import annotations

import stat
import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.session import Session, COOKIES_BASE
from tools.v4.chrome_tab import ChromeTab


def pf(label: str, ok: bool) -> None:
    print(f"  [{'PASS' if ok else 'FAIL'}] {label}")


def main() -> None:
    overall_ok = True

    print("=== F06 PoC: Cookie Persistence to Disk ===\n")

    # 1. Named session (no Chrome launch — we attach to existing Chrome)
    session = Session("linkedin-poc-f06")
    print(f"  Session created: {session!r}")

    # 2. Attach to running Chrome on port 55715
    tab = ChromeTab.open(55715)
    print(f"  ChromeTab opened on port 55715\n")

    # 3. Navigate to LinkedIn
    print("Step 3: Navigate to https://www.linkedin.com/ (wait_s=2.0)")
    tab.navigate("https://www.linkedin.com/", wait_s=2.0)
    print("  Done\n")

    # 4. Get cookies before save (all cookies — matches what save_cookies saves)
    print("Step 4: Get cookies from tab")
    cookies_before = tab.get_cookies()
    n = len(cookies_before)
    # Also check URL-scoped subset is non-empty
    cookies_url = tab.get_cookies(url="https://www.linkedin.com")
    print(f"  {n} cookies found (all), {len(cookies_url)} scoped to linkedin.com")
    ok = n > 0
    pf(f"{n} > 0", ok)
    overall_ok = overall_ok and ok
    print()

    # 5-6. Save cookies to default path
    print("Step 5-6: Save cookies to default path")
    session.save_cookies(tab)
    default_path = COOKIES_BASE / "linkedin-poc-f06.json"
    print(f"  Path written: {default_path}")

    exists = default_path.exists()
    perm_ok = False
    if exists:
        mode = stat.S_IMODE(default_path.stat().st_mode)
        perm_ok = mode == 0o600
        size = default_path.stat().st_size
    ok_file = exists and perm_ok
    pf(f"File exists + permissions 0600", ok_file)
    overall_ok = overall_ok and ok_file
    if exists:
        print(f"  File size: {size} bytes")
    print()

    # 7. Open second tab and restore cookies
    print("Step 7: Open second tab and restore cookies")
    tab2 = ChromeTab.open(55715)
    restored = session.restore_cookies(tab2)
    ok_restore = restored == n
    pf(f"restored ({restored}) == n ({n})", ok_restore)
    overall_ok = overall_ok and ok_restore
    print()

    # 8. Verify cookies are accessible in second tab (URL-scoped to confirm injection)
    print("Step 8: Get cookies from tab2 after restore")
    cookies_after = tab2.get_cookies(url="https://www.linkedin.com")
    ok_after = len(cookies_after) > 0
    pf(f"len(cookies_after) = {len(cookies_after)} > 0", ok_after)
    overall_ok = overall_ok and ok_after
    print()

    # Cleanup
    tab.close()
    tab2.close()
    print("  tabs closed\n")

    print(f"OVERALL: {'PASS' if overall_ok else 'FAIL'}")


if __name__ == "__main__":
    main()
