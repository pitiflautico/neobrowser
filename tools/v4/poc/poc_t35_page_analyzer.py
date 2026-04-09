"""
T3.5 PoC: PageAnalyzer — live test against Chrome on port 55715 with LinkedIn session.

Steps:
1. Open tab → navigate to linkedin.com/messaging/
2. Wait for conversation sidebar
3. Click first conversation
4. Wait for messages to load
5. Run PageAnalyzer heuristics: find_last_message, find_input_box, find_send_button
6. If any heuristic fails, try find_by_intent() as fallback
7. If find_input_box succeeded: type "test", then clear immediately
8. Print PASS/FAIL for each step

Usage: python3 poc_t35_page_analyzer.py [port]
  port defaults to 55715
"""
from __future__ import annotations

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.chrome_tab import ChromeTab
from tools.v4.page_analyzer import PageAnalyzer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 55715


def _step(label: str, ok: bool, detail: str = "") -> None:
    status = "PASS" if ok else "FAIL"
    suffix = f"  → {detail}" if detail else ""
    print(f"  [{status}] {label}{suffix}")


def main() -> None:
    print(f"\n=== T3.5 PageAnalyzer PoC (port {PORT}) ===\n")

    # ------------------------------------------------------------------ #
    # Step 1 — Open tab + navigate
    # ------------------------------------------------------------------ #
    print("Step 1: Open tab + navigate to LinkedIn Messaging")
    try:
        tab = ChromeTab.open(PORT)
        tab.navigate("https://www.linkedin.com/messaging/", wait_s=3.0)
        _step("open tab + navigate", True)
    except Exception as exc:
        _step("open tab + navigate", False, str(exc))
        print("\nFATAL: cannot open tab — is Chrome running on port", PORT)
        sys.exit(1)

    # ------------------------------------------------------------------ #
    # Step 2 — Wait for conversation sidebar
    # ------------------------------------------------------------------ #
    print("\nStep 2: Wait for conversation sidebar")
    found_sidebar = tab.wait_for_selector(
        ".msg-conversations-container__conversations-list", timeout_s=12.0
    )
    _step("conversation sidebar visible", found_sidebar)
    if not found_sidebar:
        print("FATAL: sidebar not found — is the LinkedIn session valid?")
        sys.exit(1)

    # ------------------------------------------------------------------ #
    # Step 3 — Click first conversation
    # ------------------------------------------------------------------ #
    print("\nStep 3: Click first conversation")
    try:
        tab.js("document.querySelector('.msg-conversation-listitem__link').click()")
        _step("click first conversation", True)
    except Exception as exc:
        _step("click first conversation", False, str(exc))

    # ------------------------------------------------------------------ #
    # Step 4 — Wait for messages
    # ------------------------------------------------------------------ #
    print("\nStep 4: Wait for message list")
    found_msgs = tab.wait_for_selector(".msg-s-event-listitem__body", timeout_s=8.0)
    _step("message list visible", found_msgs)

    # ------------------------------------------------------------------ #
    # Step 5 — PageAnalyzer heuristics
    # ------------------------------------------------------------------ #
    print("\nStep 5: PageAnalyzer heuristics")
    analyzer = PageAnalyzer()

    # 5a — snapshot
    try:
        snap = analyzer.snapshot(tab)
        _step("snapshot()", True, f"{len(snap)} nodes")
        print(f"         Sample roles: {list({n['role'] for n in snap[:20]})}")
    except Exception as exc:
        _step("snapshot()", False, str(exc))
        snap = []

    # 5b — find_last_message
    last_msg = analyzer.find_last_message(tab)
    if last_msg:
        preview = last_msg[:80].replace("\n", " ")
        _step("find_last_message()", True, repr(preview))
    else:
        _step("find_last_message()", False, "heuristic returned None — trying LLM fallback")
        last_msg_fb = analyzer.find_by_intent(tab, "last message text in the conversation")
        if last_msg_fb is not None:
            print(f"         LLM fallback backendNodeId: {last_msg_fb}")
        else:
            print("         LLM fallback: None")

    # 5c — find_input_box
    input_id = analyzer.find_input_box(tab)
    if input_id is not None:
        _step("find_input_box()", True, f"backendNodeId={input_id}")
    else:
        _step("find_input_box()", False, "heuristic returned None — trying LLM fallback")
        input_id = analyzer.find_by_intent(tab, "message text input or compose box")
        if input_id is not None:
            print(f"         LLM fallback backendNodeId: {input_id}  [layer=LLM]")
        else:
            print("         LLM fallback: None")

    # 5d — find_send_button
    send_id = analyzer.find_send_button(tab)
    if send_id is not None:
        _step("find_send_button()", True, f"backendNodeId={send_id}")
    else:
        _step("find_send_button()", False, "heuristic returned None — trying LLM fallback")
        send_id = analyzer.find_by_intent(tab, "send message button")
        if send_id is not None:
            print(f"         LLM fallback backendNodeId: {send_id}  [layer=LLM]")
        else:
            print("         LLM fallback: None")

    # ------------------------------------------------------------------ #
    # Step 6 — type_in_node + clear
    # ------------------------------------------------------------------ #
    print("\nStep 6: type_in_node + clear")
    if input_id is not None:
        typed = analyzer.type_in_node(tab, input_id, "test")
        _step("type_in_node('test')", typed)
        if typed:
            # Clear immediately so we don't accidentally send anything
            try:
                tab.js("document.activeElement.innerHTML=''")
                _step("clear active element", True)
            except Exception as exc:
                _step("clear active element", False, str(exc))
    else:
        _step("type_in_node (skipped — no input found)", False)

    # ------------------------------------------------------------------ #
    # Summary
    # ------------------------------------------------------------------ #
    print("\n=== Summary ===")
    print(f"  AX snapshot nodes : {len(snap)}")
    print(f"  last_message      : {repr(last_msg[:60]) if last_msg else 'None'}")
    print(f"  input_box id      : {input_id}")
    print(f"  send_button id    : {send_id}")

    overall = bool(snap and (last_msg or input_id or send_id))
    print(f"\nVERDICT: {'PASS' if overall else 'FAIL'}")


if __name__ == "__main__":
    main()
