"""
tools/v4/poc/poc_f10_playbook.py

F10 — ActionRecorder + Playbook Engine PoC

Validates record/save/load/replay cycle against real Chrome.
Uses example.com (no auth needed) to test the full stack.

Prerequisites:
  - Chrome running on port 55715
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f10_playbook.py
"""
from __future__ import annotations

import shutil
import sys
import os
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from pathlib import Path
from tools.v4.chrome_tab import ChromeTab
from tools.v4.page_analyzer import PageAnalyzer
from tools.v4.playbook import ActionRecorder, PlaybookStore, PlaybookRunner, Step

CHROME_PORT = 55715
TEST_DOMAIN = "example.com"
TEST_TASK = "poc-navigate-and-wait"
STORE_BASE = Path("/tmp/poc_f10_playbooks")

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
    print("F10 ActionRecorder + Playbook Engine PoC")
    print("=" * 60)

    # Clean test store
    if STORE_BASE.exists():
        shutil.rmtree(STORE_BASE)

    store = PlaybookStore(base=STORE_BASE)
    recorder = ActionRecorder()
    analyzer = PageAnalyzer(cache_ttl_s=5.0)
    runner = PlaybookRunner(wait_after_navigate_s=2.0)

    tab = ChromeTab.open(CHROME_PORT)
    try:
        # ------------------------------------------------------------------ #
        # Phase 1: Record a playbook manually
        # ------------------------------------------------------------------ #
        print("\n[1] Recording playbook (navigate + wait_selector)…")
        recorder.record(Step("navigate", {"url": "https://example.com"}))
        tab.navigate("https://example.com", wait_s=2.0)

        recorder.record(Step("wait_selector", {"selector": "h1", "timeout_s": 5.0}))
        found = tab.wait_for_selector("h1", timeout_s=5.0)
        check("h1 found on example.com", found)

        steps = recorder.get_playbook()
        check("recorder has 2 steps", len(steps) == 2, f"count={len(steps)}")

        # ------------------------------------------------------------------ #
        # Phase 2: Save playbook
        # ------------------------------------------------------------------ #
        print("\n[2] Saving playbook to disk…")
        store.save(TEST_DOMAIN, TEST_TASK, steps)
        pb_path = STORE_BASE / TEST_DOMAIN / f"{TEST_TASK}.json"
        check("playbook file exists", pb_path.exists(), str(pb_path))
        check("file permissions 0600", (pb_path.stat().st_mode & 0o777) == 0o600)
        check("list_tasks() finds it", TEST_TASK in store.list_tasks(TEST_DOMAIN))

        # ------------------------------------------------------------------ #
        # Phase 3: Load playbook
        # ------------------------------------------------------------------ #
        print("\n[3] Loading playbook from disk…")
        loaded = store.load(TEST_DOMAIN, TEST_TASK)
        check("load() returns list", loaded is not None)
        check("loaded 2 steps", loaded is not None and len(loaded) == 2,
              f"count={len(loaded) if loaded else 0}")

        # ------------------------------------------------------------------ #
        # Phase 4: Replay playbook — measure CDP calls saved
        # ------------------------------------------------------------------ #
        print("\n[4] Replaying playbook (should reuse cached AX)…")
        # Navigate away first so replay has real work to do
        tab.navigate("about:blank", wait_s=0.5)

        t0 = time.monotonic()
        ok, first_fail = runner.run(tab, loaded, analyzer)
        replay_ms = (time.monotonic() - t0) * 1000

        check("runner.run() returns ok=True", ok is True, f"first_fail={first_fail}")
        check("replay completed", first_fail == -1)
        print(f"    Replay took {replay_ms:.0f}ms")

        # ------------------------------------------------------------------ #
        # Phase 5: Simulate stale backendNodeId → fallback re-discovery
        # ------------------------------------------------------------------ #
        print("\n[5] Simulating stale click_node step (fallback path)…")
        stale_steps = [
            Step("navigate", {"url": "https://example.com"}),
            # Deliberately stale backendNodeId
            Step(
                "click_node",
                {"backend_node_id": 999999, "role": "heading", "name": "Example Domain"},
                fallback={"role": "heading", "name": "Example Domain"},
            ),
        ]

        # Mock analyzer.find_by_intent to return a valid id
        original_find = analyzer.find_by_intent
        rediscovery_calls = [0]

        def mock_find(t, intent):
            rediscovery_calls[0] += 1
            # Return a valid node id via JS
            result = t.send("DOM.getDocument", {})
            return result.get("root", {}).get("backendNodeId", 1)

        analyzer.find_by_intent = mock_find
        try:
            ok2, ff2 = runner.run(tab, stale_steps, analyzer)
        finally:
            analyzer.find_by_intent = original_find

        check(
            "fallback triggered (find_by_intent called)",
            rediscovery_calls[0] >= 1,
            f"calls={rediscovery_calls[0]}",
        )
        check(
            "stale backendNodeId updated in step",
            stale_steps[1].params["backend_node_id"] != 999999,
            f"new_id={stale_steps[1].params.get('backend_node_id')}",
        )

    finally:
        tab.close()
        # Cleanup
        if STORE_BASE.exists():
            shutil.rmtree(STORE_BASE)
        print("\n    Test store cleaned up.")

    print("\n" + "=" * 60)
    if overall_pass:
        print("\033[32mOVERALL: PASS\033[0m")
    else:
        print("\033[31mOVERALL: FAIL\033[0m")
    print("=" * 60)
    sys.exit(0 if overall_pass else 1)


if __name__ == "__main__":
    main()
