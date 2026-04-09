"""
tools/v4/poc/poc_f11_lifecycle.py

F11 — Data Lifecycle Manager PoC

Validates DataLifecycleManager against real filesystem fixtures.
Creates isolated test fixtures under /tmp/neorender-lifecycle-poc/,
runs compaction, verifies results, cleans up.

Does NOT touch ~/.neorender/.

Usage:
    python3 tools/v4/poc/poc_f11_lifecycle.py
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

import tools.v4.lifecycle as lifecycle_mod
from tools.v4.lifecycle import DataLifecycleManager

PASS = "\033[32m[PASS]\033[0m"
FAIL = "\033[31m[FAIL]\033[0m"
overall_pass = True

POC_BASE = Path("/tmp/neorender-lifecycle-poc")


def check(label: str, condition: bool) -> None:
    global overall_pass
    print(f"  {PASS if condition else FAIL} {label}")
    if not condition:
        overall_pass = False


def setup_fixtures(base: Path) -> None:
    """Create test fixtures under base/."""
    # Playbooks
    pb = base / "playbooks" / "linkedin.com"
    pb.mkdir(parents=True)
    old_pb = pb / "send_message.json"
    fresh_pb = pb / "browse_feed.json"
    old_pb.write_text(json.dumps([{"action": "navigate", "params": {}}]))
    fresh_pb.write_text(json.dumps([{"action": "navigate", "params": {}}]))
    _age_file(old_pb, 35 * 86400)   # 35 days → stale
    _age_file(fresh_pb, 2 * 86400)  # 2 days  → keep

    # Cookies
    ck = base / "cookies"
    ck.mkdir(parents=True)
    stale_ck = ck / "linkedin.json"
    fresh_ck = ck / "github.json"
    stale_ck.write_text(json.dumps([{"name": "session", "value": "abc"}]))
    fresh_ck.write_text(json.dumps([{"name": "token", "value": "xyz"}]))
    _age_file(stale_ck, 10 * 86400)  # 10 days → stale
    _age_file(fresh_ck, 1 * 86400)   # 1 day   → keep

    # Profiles
    pr = base / "profiles"
    pr.mkdir(parents=True)
    active_p = pr / "linkedin"
    idle_p = pr / "old-research"
    active_p.mkdir()
    idle_p.mkdir()
    (active_p / "Cookies").write_bytes(b"x" * 1024)
    (idle_p / "Cookies").write_bytes(b"x" * 512)
    _age_file(idle_p, 30 * 3600)    # 30 hours → should delete
    _age_file(active_p, 30 * 3600)  # same age, but active → keep

    # Logs
    lg = base / "logs"
    lg.mkdir(parents=True)
    big_log = lg / "session.jsonl"
    small_log = lg / "debug.jsonl"
    big_log.write_bytes(b'{"event":"navigate"}\n' * 100_000)  # ~2MB
    small_log.write_bytes(b'{"event":"click"}\n' * 10)


def _age_file(path: Path, age_seconds: float) -> None:
    t = time.time() - age_seconds
    os.utime(path, (t, t))


def patch_lifecycle(base: Path) -> None:
    """Redirect lifecycle module paths to poc base."""
    lifecycle_mod.NEORENDER_BASE = base
    lifecycle_mod.PLAYBOOKS_BASE = base / "playbooks"
    lifecycle_mod.COOKIES_BASE = base / "cookies"
    lifecycle_mod.PROFILES_BASE = base / "profiles"
    lifecycle_mod.LOGS_BASE = base / "logs"


def main() -> None:
    global overall_pass

    print("=" * 60)
    print("F11 Data Lifecycle Manager PoC")
    print("=" * 60)

    # Setup
    if POC_BASE.exists():
        shutil.rmtree(POC_BASE)
    POC_BASE.mkdir(parents=True)

    print(f"\n[1] Creating fixtures in {POC_BASE}…")
    setup_fixtures(POC_BASE)
    patch_lifecycle(POC_BASE)
    print("    Fixtures created.")

    # ------------------------------------------------------------------ #
    # Check 1: compact_playbooks removes > 30d, keeps < 30d
    # ------------------------------------------------------------------ #
    print("\n[2] compact_playbooks(ttl=30d)…")
    mgr = DataLifecycleManager(
        playbook_ttl_days=30,
        cookie_ttl_days=7,
        profile_ttl_hours=24,
        log_max_bytes=1 * 1024 * 1024,  # 1MB
        log_max_files=3,
        active_profiles={"linkedin"},
    )
    pb_count = mgr.compact_playbooks()
    pb_base = POC_BASE / "playbooks" / "linkedin.com"
    check("compact_playbooks() removed 1 file", pb_count == 1)
    check("stale playbook deleted", not (pb_base / "send_message.json").exists())
    check("fresh playbook kept", (pb_base / "browse_feed.json").exists())

    # ------------------------------------------------------------------ #
    # Check 2: compact_cookies removes stale cookie file
    # ------------------------------------------------------------------ #
    print("\n[3] compact_cookies(ttl=7d)…")
    ck_count = mgr.compact_cookies()
    ck_base = POC_BASE / "cookies"
    check("compact_cookies() removed 1 file", ck_count == 1)
    check("stale cookie deleted", not (ck_base / "linkedin.json").exists())
    check("fresh cookie kept", (ck_base / "github.json").exists())

    # ------------------------------------------------------------------ #
    # Check 3: compact_profiles skips active, removes idle
    # ------------------------------------------------------------------ #
    print("\n[4] compact_profiles(active={'linkedin'})…")
    pr_count = mgr.compact_profiles()
    pr_base = POC_BASE / "profiles"
    check("compact_profiles() removed 1 directory", pr_count == 1)
    check("active profile 'linkedin' kept", (pr_base / "linkedin").exists())
    check("idle profile 'old-research' deleted", not (pr_base / "old-research").exists())

    # ------------------------------------------------------------------ #
    # Check 4: rotate_logs rotates oversized log file
    # ------------------------------------------------------------------ #
    print("\n[5] rotate_logs(max=1MB)…")
    lg_count = mgr.rotate_logs()
    lg_base = POC_BASE / "logs"
    check("rotate_logs() rotated 1 file", lg_count == 1)
    check("session.jsonl recreated (empty)", (lg_base / "session.jsonl").stat().st_size == 0)
    check("session.1.jsonl contains rotated data", (lg_base / "session.1.jsonl").stat().st_size > 1_000_000)
    check("small debug.jsonl untouched", (lg_base / "debug.jsonl").stat().st_size < 1 * 1024 * 1024)

    # ------------------------------------------------------------------ #
    # Check 5: run_all returns CompactionReport with bytes_freed > 0
    # ------------------------------------------------------------------ #
    print("\n[6] Fresh fixture + run_all()…")
    shutil.rmtree(POC_BASE)
    POC_BASE.mkdir(parents=True)
    setup_fixtures(POC_BASE)
    patch_lifecycle(POC_BASE)

    mgr2 = DataLifecycleManager(
        playbook_ttl_days=30,
        cookie_ttl_days=7,
        profile_ttl_hours=24,
        log_max_bytes=1 * 1024 * 1024,
        active_profiles={"linkedin"},
    )
    report = mgr2.run_all()
    print(f"    {report}")
    check("report.deleted_playbooks >= 1", report.deleted_playbooks >= 1)
    check("report.deleted_cookies >= 1", report.deleted_cookies >= 1)
    check("report.deleted_profiles >= 1", report.deleted_profiles >= 1)
    check("report.rotated_logs >= 1", report.rotated_logs >= 1)
    check("report.bytes_freed > 0", report.bytes_freed > 0)
    check("report.errors == []", report.errors == [])

    # ------------------------------------------------------------------ #
    # Check 6: dry_run=True — no files deleted
    # ------------------------------------------------------------------ #
    print("\n[7] dry_run=True on fresh fixture…")
    shutil.rmtree(POC_BASE)
    POC_BASE.mkdir(parents=True)
    setup_fixtures(POC_BASE)
    patch_lifecycle(POC_BASE)

    before_stale_pb = (POC_BASE / "playbooks" / "linkedin.com" / "send_message.json").exists()
    before_stale_ck = (POC_BASE / "cookies" / "linkedin.json").exists()

    dry_mgr = DataLifecycleManager(
        playbook_ttl_days=30,
        cookie_ttl_days=7,
        profile_ttl_hours=24,
        log_max_bytes=1 * 1024 * 1024,
        active_profiles={"linkedin"},
        dry_run=True,
    )
    dry_report = dry_mgr.run_all()
    print(f"    {dry_report}")
    check("dry_run: stale playbook still exists", (POC_BASE / "playbooks" / "linkedin.com" / "send_message.json").exists())
    check("dry_run: stale cookie still exists", (POC_BASE / "cookies" / "linkedin.json").exists())
    check("dry_run: idle profile still exists", (POC_BASE / "profiles" / "old-research").exists())
    check("dry_run: report shows work would be done", dry_report.deleted_playbooks >= 1 or dry_report.deleted_cookies >= 1)

    # ------------------------------------------------------------------ #
    # Cleanup
    # ------------------------------------------------------------------ #
    shutil.rmtree(POC_BASE)
    print("\n    Fixtures cleaned up.")

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
