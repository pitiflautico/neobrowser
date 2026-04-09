"""
tools/v4/poc/poc_t3_linkedin_live.py

Live smoke test for LinkedInNavigator against a real Chrome on port 55715.

Steps:
  1. Open Chrome tab on port 55715
  2. Navigate to messaging, click Toni's conversation (first in sidebar)
  3. get_last_message() → must return something
  4. find_input_box → must return a node_id
  5. find_send_button → must return a node_id

Run:
  python3 tools/v4/poc/poc_t3_linkedin_live.py
"""
from __future__ import annotations

import sys

PORT = 55715

_PASS = "\033[32mPASS\033[0m"
_FAIL = "\033[31mFAIL\033[0m"


def check(label: str, condition: bool, detail: str = "") -> bool:
    status = _PASS if condition else _FAIL
    suffix = f"  ({detail})" if detail else ""
    print(f"  [{status}] {label}{suffix}")
    return condition


def main() -> int:
    from tools.v4.chrome_tab import ChromeTab
    from tools.v4.page_analyzer import PageAnalyzer
    from tools.v4.linkedin_nav import LinkedInNavigator

    print(f"\nNeoBrowser V4 — LinkedIn live PoC (port {PORT})")
    print("=" * 55)

    # --- Step 1: open tab ---
    print("\n[1] Opening Chrome tab")
    try:
        tab = ChromeTab.open(PORT)
    except Exception as exc:
        print(f"  [{_FAIL}] Could not open tab: {exc}")
        return 1
    check("Tab opened", True, f"tab_id={tab._tab_id}")

    analyzer = PageAnalyzer()
    nav = LinkedInNavigator(analyzer=analyzer)
    all_ok = True

    # --- Step 2: open first conversation ---
    print("\n[2] Navigate to messaging + click first conversation")
    try:
        ok = nav.open_thread(tab, "toni-first-in-sidebar")
    except Exception as exc:
        ok = False
        print(f"  exception: {exc}")
    all_ok &= check("open_thread() returned True", ok)

    # --- Step 3: get_last_message ---
    print("\n[3] get_last_message()")
    try:
        msg = nav.get_last_message(tab)
    except Exception as exc:
        msg = None
        print(f"  exception: {exc}")
    all_ok &= check("last message is not None", msg is not None, repr(msg))

    # --- Step 4: find_input_box ---
    print("\n[4] find_input_box()")
    try:
        input_node = analyzer.find_input_box(tab)
    except Exception as exc:
        input_node = None
        print(f"  exception: {exc}")
    all_ok &= check("find_input_box returned node_id", input_node is not None,
                    f"backendNodeId={input_node}")

    # --- Step 5: find_send_button ---
    print("\n[5] find_send_button()")
    try:
        send_node = analyzer.find_send_button(tab)
    except Exception as exc:
        send_node = None
        print(f"  exception: {exc}")
    all_ok &= check("find_send_button returned node_id", send_node is not None,
                    f"backendNodeId={send_node}")

    # --- Summary ---
    print("\n" + "=" * 55)
    if all_ok:
        print(f"  OVERALL: {_PASS}")
    else:
        print(f"  OVERALL: {_FAIL}")

    try:
        tab.close()
    except Exception:
        pass

    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
