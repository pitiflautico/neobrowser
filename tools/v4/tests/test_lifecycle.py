"""
tools/v4/tests/test_lifecycle.py

Unit tests for F11 — DataLifecycleManager.

All tests use tmp_path (pytest fixture) — no ~/.neorender/ touched.
We monkey-patch NEORENDER_BASE, PROFILES_BASE, COOKIES_BASE, and PLAYBOOKS_BASE
inside DataLifecycleManager so tests are fully isolated.
"""
from __future__ import annotations

import json
import time
from pathlib import Path

import pytest

import tools.v4.lifecycle as lifecycle_mod
from tools.v4.lifecycle import CompactionReport, DataLifecycleManager


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture()
def base(tmp_path, monkeypatch):
    """
    Redirect all lifecycle paths to a tmp directory.
    Returns the fake NEORENDER_BASE.
    """
    fake_base = tmp_path / "neorender"
    fake_base.mkdir()

    fake_playbooks = fake_base / "playbooks"
    fake_cookies = fake_base / "cookies"
    fake_profiles = fake_base / "profiles"
    fake_logs = fake_base / "logs"

    for d in (fake_playbooks, fake_cookies, fake_profiles, fake_logs):
        d.mkdir()

    monkeypatch.setattr(lifecycle_mod, "NEORENDER_BASE", fake_base)
    monkeypatch.setattr(lifecycle_mod, "PLAYBOOKS_BASE", fake_playbooks)
    monkeypatch.setattr(lifecycle_mod, "LOGS_BASE", fake_logs)

    # Patch COOKIES_BASE and PROFILES_BASE in the module namespace
    import tools.v4.session as session_mod
    import tools.v4.chrome_process as cp_mod
    monkeypatch.setattr(session_mod, "COOKIES_BASE", fake_cookies)
    monkeypatch.setattr(cp_mod, "PROFILES_BASE", fake_profiles)

    # Also patch the imported names in lifecycle_mod
    monkeypatch.setattr(lifecycle_mod, "COOKIES_BASE", fake_cookies)
    monkeypatch.setattr(lifecycle_mod, "PROFILES_BASE", fake_profiles)

    return fake_base


def _age_file(path: Path, age_seconds: float) -> None:
    """Set mtime + atime to now - age_seconds."""
    t = time.time() - age_seconds
    import os
    os.utime(path, (t, t))


# ---------------------------------------------------------------------------
# 1. compact_playbooks — deletes old, keeps recent
# ---------------------------------------------------------------------------


def test_compact_playbooks_deletes_old(base, monkeypatch):
    playbooks = base / "playbooks"
    (playbooks / "linkedin").mkdir()
    old = playbooks / "linkedin" / "send_message.json"
    new = playbooks / "linkedin" / "browse_feed.json"
    old.write_text(json.dumps([]))
    new.write_text(json.dumps([]))

    _age_file(old, 31 * 86400)   # 31 days old → should delete
    _age_file(new, 1 * 86400)    # 1 day old   → keep

    mgr = DataLifecycleManager(playbook_ttl_days=30)
    count = mgr.compact_playbooks()

    assert count == 1
    assert not old.exists()
    assert new.exists()


def test_compact_playbooks_returns_count(base):
    playbooks = base / "playbooks"
    (playbooks / "domain").mkdir()
    for i in range(3):
        f = playbooks / "domain" / f"task{i}.json"
        f.write_text("{}")
        _age_file(f, 40 * 86400)  # all stale

    mgr = DataLifecycleManager(playbook_ttl_days=30)
    assert mgr.compact_playbooks() == 3


# ---------------------------------------------------------------------------
# 3. compact_cookies — deletes stale, keeps recent
# ---------------------------------------------------------------------------


def test_compact_cookies_deletes_stale(base):
    cookies = base / "cookies"
    stale = cookies / "linkedin.json"
    fresh = cookies / "github.json"
    stale.write_text("[]")
    fresh.write_text("[]")

    _age_file(stale, 8 * 86400)  # 8 days → stale
    _age_file(fresh, 2 * 86400)  # 2 days → keep

    mgr = DataLifecycleManager(cookie_ttl_days=7)
    count = mgr.compact_cookies()

    assert count == 1
    assert not stale.exists()
    assert fresh.exists()


def test_compact_cookies_keeps_recent(base):
    cookies = base / "cookies"
    f = cookies / "fresh.json"
    f.write_text("[]")
    _age_file(f, 1 * 86400)  # 1 day → keep

    mgr = DataLifecycleManager(cookie_ttl_days=7)
    assert mgr.compact_cookies() == 0
    assert f.exists()


# ---------------------------------------------------------------------------
# 4. compact_profiles — skips active_profiles
# ---------------------------------------------------------------------------


def test_compact_profiles_skips_active(base):
    profiles = base / "profiles"
    active_dir = profiles / "linkedin"
    idle_dir = profiles / "old-profile"
    active_dir.mkdir()
    idle_dir.mkdir()

    _age_file(active_dir, 48 * 3600)
    _age_file(idle_dir, 48 * 3600)

    mgr = DataLifecycleManager(profile_ttl_hours=24, active_profiles={"linkedin"})
    count = mgr.compact_profiles()

    assert count == 1
    assert active_dir.exists()
    assert not idle_dir.exists()


# ---------------------------------------------------------------------------
# 5. compact_profiles — does not delete recent profiles
# ---------------------------------------------------------------------------


def test_compact_profiles_keeps_recent(base):
    profiles = base / "profiles"
    fresh = profiles / "fresh-profile"
    fresh.mkdir()
    _age_file(fresh, 1 * 3600)  # 1 hour old → keep

    mgr = DataLifecycleManager(profile_ttl_hours=24)
    assert mgr.compact_profiles() == 0
    assert fresh.exists()


# ---------------------------------------------------------------------------
# 6. rotate_logs — rotates files over max_bytes
# ---------------------------------------------------------------------------


def test_rotate_logs_rotates_oversized(base):
    logs = base / "logs"
    log_file = logs / "session.jsonl"
    log_file.write_bytes(b"x" * (2 * 1024 * 1024))  # 2MB

    mgr = DataLifecycleManager(log_max_bytes=1 * 1024 * 1024, log_max_files=5)
    count = mgr.rotate_logs()

    assert count == 1
    # Original file recreated empty (for ongoing writes)
    assert log_file.exists()
    assert log_file.stat().st_size == 0
    # Rotated file exists at .1
    rotated = logs / "session.1.jsonl"
    assert rotated.exists()
    assert rotated.stat().st_size == 2 * 1024 * 1024


# ---------------------------------------------------------------------------
# 7. rotate_logs — does not touch files under limit
# ---------------------------------------------------------------------------


def test_rotate_logs_keeps_small_files(base):
    logs = base / "logs"
    small = logs / "small.jsonl"
    small.write_bytes(b"x" * 100)

    mgr = DataLifecycleManager(log_max_bytes=1 * 1024 * 1024)
    assert mgr.rotate_logs() == 0
    assert small.exists()
    assert small.stat().st_size == 100


# ---------------------------------------------------------------------------
# 8. run_all — returns CompactionReport with correct totals
# ---------------------------------------------------------------------------


def test_run_all_returns_report(base):
    playbooks = base / "playbooks"
    cookies = base / "cookies"
    (playbooks / "d").mkdir()
    old_pb = playbooks / "d" / "old.json"
    old_ck = cookies / "old.json"
    old_pb.write_text("{}")
    old_ck.write_text("[]")
    _age_file(old_pb, 40 * 86400)
    _age_file(old_ck, 10 * 86400)

    mgr = DataLifecycleManager(playbook_ttl_days=30, cookie_ttl_days=7)
    report = mgr.run_all()

    assert isinstance(report, CompactionReport)
    assert report.deleted_playbooks == 1
    assert report.deleted_cookies == 1
    assert report.bytes_freed > 0


# ---------------------------------------------------------------------------
# 9. run_all — continues after single file delete error
# ---------------------------------------------------------------------------


def test_run_all_continues_after_error(base, monkeypatch):
    cookies = base / "cookies"
    bad = cookies / "bad.json"
    bad.write_text("[]")
    _age_file(bad, 10 * 86400)

    # Make unlink raise an error
    original_unlink = Path.unlink

    def flaky_unlink(self, missing_ok=False):
        if self.name == "bad.json":
            raise PermissionError("locked")
        original_unlink(self, missing_ok=missing_ok)

    monkeypatch.setattr(Path, "unlink", flaky_unlink)

    mgr = DataLifecycleManager(cookie_ttl_days=7)
    report = mgr.run_all()

    assert len(report.errors) >= 1
    assert any("bad.json" in e or "locked" in e for e in report.errors)


# ---------------------------------------------------------------------------
# 10. Path outside NEORENDER_BASE raises ValueError
# ---------------------------------------------------------------------------


def test_path_traversal_raises(base):
    mgr = DataLifecycleManager()
    outside = Path("/tmp/evil_file.txt")
    with pytest.raises(ValueError, match="Path traversal"):
        mgr._safe_path(outside)


# ---------------------------------------------------------------------------
# 11. dry_run=True logs but does not delete
# ---------------------------------------------------------------------------


def test_dry_run_does_not_delete(base):
    cookies = base / "cookies"
    f = cookies / "profile.json"
    f.write_text("[]")
    _age_file(f, 10 * 86400)

    mgr = DataLifecycleManager(cookie_ttl_days=7, dry_run=True)
    count = mgr.compact_cookies()

    # count returned as if it would delete
    assert count == 1
    # but file still exists
    assert f.exists()


# ---------------------------------------------------------------------------
# 12. disk_usage — returns correct byte count
# ---------------------------------------------------------------------------


def test_disk_usage_sums_bytes(tmp_path):
    d = tmp_path / "data"
    d.mkdir()
    (d / "a.txt").write_bytes(b"x" * 1000)
    (d / "b.txt").write_bytes(b"x" * 2000)
    sub = d / "sub"
    sub.mkdir()
    (sub / "c.txt").write_bytes(b"x" * 500)

    assert DataLifecycleManager.disk_usage(d) == 3500


def test_disk_usage_nonexistent_returns_zero(tmp_path):
    assert DataLifecycleManager.disk_usage(tmp_path / "missing") == 0
