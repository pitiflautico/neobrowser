"""
tools/v4/lifecycle.py

F11 — Data Lifecycle Manager (Deletion + Compaction)

Manages all persistent data under ~/.neorender/:
  - Playbooks:      ~/.neorender/playbooks/{domain}/{task}.json
  - Cookies:        ~/.neorender/cookies/{profile}.json
  - Chrome profiles:~/.neorender/profiles/{name}/
  - Log exports:    ~/.neorender/logs/*.jsonl (rotation)

Safety invariants:
  - No deletion outside NEORENDER_BASE (is_relative_to() guard)
  - active_profiles set prevents live-session profiles from being deleted
  - Per-file errors are collected, not raised (non-fatal)
  - dry_run=True logs what would happen, touches nothing
"""
from __future__ import annotations

import logging
import os
import shutil
import time
from dataclasses import dataclass, field
from pathlib import Path

from tools.v4.chrome_process import PROFILES_BASE
from tools.v4.session import COOKIES_BASE

log = logging.getLogger(__name__)

NEORENDER_BASE = Path.home() / ".neorender"
PLAYBOOKS_BASE = NEORENDER_BASE / "playbooks"
LOGS_BASE = NEORENDER_BASE / "logs"

_DEFAULT_PLAYBOOK_TTL_DAYS = 30
_DEFAULT_COOKIE_TTL_DAYS = 7
_DEFAULT_PROFILE_TTL_HOURS = 24
_DEFAULT_LOG_MAX_BYTES = 10 * 1024 * 1024  # 10 MB
_DEFAULT_LOG_MAX_FILES = 5


@dataclass
class CompactionReport:
    deleted_playbooks: int = 0
    deleted_cookies: int = 0
    deleted_profiles: int = 0
    rotated_logs: int = 0
    bytes_freed: int = 0
    errors: list[str] = field(default_factory=list)

    def __str__(self) -> str:
        return (
            f"CompactionReport("
            f"playbooks={self.deleted_playbooks}, "
            f"cookies={self.deleted_cookies}, "
            f"profiles={self.deleted_profiles}, "
            f"logs_rotated={self.rotated_logs}, "
            f"freed={self.bytes_freed:,}B, "
            f"errors={len(self.errors)})"
        )


class DataLifecycleManager:
    """
    Manages deletion and compaction of all ~/.neorender/ persistent data.

    Usage:
        mgr = DataLifecycleManager(active_profiles={"linkedin", "default"})
        report = mgr.run_all()
        print(report)

    dry_run=True: logs what would be deleted without deleting anything.
    """

    def __init__(
        self,
        playbook_ttl_days: int = _DEFAULT_PLAYBOOK_TTL_DAYS,
        cookie_ttl_days: int = _DEFAULT_COOKIE_TTL_DAYS,
        profile_ttl_hours: int = _DEFAULT_PROFILE_TTL_HOURS,
        log_max_bytes: int = _DEFAULT_LOG_MAX_BYTES,
        log_max_files: int = _DEFAULT_LOG_MAX_FILES,
        active_profiles: set[str] | None = None,
        dry_run: bool = False,
    ) -> None:
        self.playbook_ttl_s = playbook_ttl_days * 86400.0
        self.cookie_ttl_s = cookie_ttl_days * 86400.0
        self.profile_ttl_s = profile_ttl_hours * 3600.0
        self.log_max_bytes = log_max_bytes
        self.log_max_files = log_max_files
        self.active_profiles: set[str] = active_profiles or set()
        self.dry_run = dry_run

    # ------------------------------------------------------------------
    # Safety
    # ------------------------------------------------------------------

    def _safe_path(self, path: Path) -> Path:
        """
        Resolve path and assert it is under NEORENDER_BASE.
        Raises ValueError on path traversal attempt.
        """
        resolved = path.resolve()
        base = NEORENDER_BASE.resolve()
        if not resolved.is_relative_to(base):
            raise ValueError(
                f"Path traversal detected: {path!r} resolves to {resolved!r} "
                f"which is outside {base!r}"
            )
        return resolved

    def _delete_file(self, path: Path, report: CompactionReport) -> int:
        """
        Delete a single file. Returns bytes freed (0 on error).
        Appends error string to report.errors on failure.
        """
        self._safe_path(path)
        try:
            size = path.stat().st_size
            log.debug("delete_file%s: %s (%d B)", " [DRY]" if self.dry_run else "", path, size)
            if not self.dry_run:
                path.unlink()
            return size
        except Exception as exc:
            msg = f"Failed to delete {path}: {exc}"
            log.warning(msg)
            report.errors.append(msg)
            return 0

    def _delete_dir(self, path: Path, report: CompactionReport) -> int:
        """
        Recursively delete a directory. Returns bytes freed (0 on error).
        """
        self._safe_path(path)
        try:
            size = self.disk_usage(path)
            log.debug("delete_dir%s: %s (%d B)", " [DRY]" if self.dry_run else "", path, size)
            if not self.dry_run:
                shutil.rmtree(path)
            return size
        except Exception as exc:
            msg = f"Failed to delete directory {path}: {exc}"
            log.warning(msg)
            report.errors.append(msg)
            return 0

    # ------------------------------------------------------------------
    # Targeted operations
    # ------------------------------------------------------------------

    def compact_playbooks(self, report: CompactionReport | None = None) -> int:
        """
        Delete .json playbook files not accessed in playbook_ttl_days.
        Returns count of deleted files.
        """
        _report = report or CompactionReport()
        if not PLAYBOOKS_BASE.exists():
            return 0

        now = time.time()
        count = 0
        for json_file in PLAYBOOKS_BASE.rglob("*.json"):
            try:
                self._safe_path(json_file)
                mtime = json_file.stat().st_mtime
                atime = json_file.stat().st_atime
                # Use most recent of mtime/atime
                last_used = max(mtime, atime)
                age_s = now - last_used
                if age_s > self.playbook_ttl_s:
                    freed = self._delete_file(json_file, _report)
                    _report.bytes_freed += freed
                    _report.deleted_playbooks += 1
                    count += 1
            except ValueError:
                # Path traversal — skip
                continue
            except Exception as exc:
                _report.errors.append(f"compact_playbooks scan error {json_file}: {exc}")
        return count

    def compact_cookies(self, report: CompactionReport | None = None) -> int:
        """
        Delete cookie .json files not modified in cookie_ttl_days.
        Returns count of deleted files.
        """
        _report = report or CompactionReport()
        if not COOKIES_BASE.exists():
            return 0

        now = time.time()
        count = 0
        for json_file in COOKIES_BASE.glob("*.json"):
            try:
                self._safe_path(json_file)
                age_s = now - json_file.stat().st_mtime
                if age_s > self.cookie_ttl_s:
                    freed = self._delete_file(json_file, _report)
                    _report.bytes_freed += freed
                    _report.deleted_cookies += 1
                    count += 1
            except ValueError:
                continue
            except Exception as exc:
                _report.errors.append(f"compact_cookies scan error {json_file}: {exc}")
        return count

    def compact_profiles(
        self,
        active_profiles: set[str] | None = None,
        report: CompactionReport | None = None,
    ) -> int:
        """
        Delete Chrome profile directories idle > profile_ttl_hours.

        NEVER deletes a profile whose name is in active_profiles
        (constructor set merged with argument set).
        Returns count of deleted directories.
        """
        _report = report or CompactionReport()
        _active = self.active_profiles | (active_profiles or set())

        if not PROFILES_BASE.exists():
            return 0

        now = time.time()
        count = 0
        for profile_dir in PROFILES_BASE.iterdir():
            if not profile_dir.is_dir():
                continue
            name = profile_dir.name
            if name in _active:
                log.debug("compact_profiles: skipping active profile %r", name)
                continue
            try:
                self._safe_path(profile_dir)
                # Use mtime of the directory itself as last-used proxy
                age_s = now - profile_dir.stat().st_mtime
                if age_s > self.profile_ttl_s:
                    freed = self._delete_dir(profile_dir, _report)
                    _report.bytes_freed += freed
                    _report.deleted_profiles += 1
                    count += 1
            except ValueError:
                continue
            except Exception as exc:
                _report.errors.append(f"compact_profiles scan error {profile_dir}: {exc}")
        return count

    def rotate_logs(self, report: CompactionReport | None = None) -> int:
        """
        Rotate log files in LOGS_BASE that exceed log_max_bytes.
        Keeps log_max_files most-recent rotated files per base name.
        Returns count of rotated files.
        """
        _report = report or CompactionReport()
        if not LOGS_BASE.exists():
            return 0

        count = 0
        for log_file in LOGS_BASE.glob("*.jsonl"):
            try:
                self._safe_path(log_file)
                size = log_file.stat().st_size
                if size <= self.log_max_bytes:
                    continue

                # Rotate: rename current to .1, shift older ones, drop overflow
                log.debug(
                    "rotate_logs%s: %s (%d B > %d B limit)",
                    " [DRY]" if self.dry_run else "",
                    log_file,
                    size,
                    self.log_max_bytes,
                )
                if not self.dry_run:
                    self._rotate_single_log(log_file, _report)
                _report.rotated_logs += 1
                count += 1
            except ValueError:
                continue
            except Exception as exc:
                _report.errors.append(f"rotate_logs error {log_file}: {exc}")
        return count

    def _rotate_single_log(self, log_file: Path, report: CompactionReport) -> None:
        """
        Rotate log_file.jsonl → log_file.1.jsonl → ... up to log_max_files.
        Deletes files beyond log_max_files.
        """
        stem = log_file.stem  # e.g. "session"
        parent = log_file.parent

        # Collect existing rotated files: session.1.jsonl, session.2.jsonl, …
        rotated: list[tuple[int, Path]] = []
        for f in parent.glob(f"{stem}.*.jsonl"):
            parts = f.name.split(".")
            # stem.N.jsonl → parts = [stem, "N", "jsonl"]
            if len(parts) == 3 and parts[1].isdigit():
                rotated.append((int(parts[1]), f))
        rotated.sort(key=lambda x: x[0], reverse=True)

        # Shift: session.N.jsonl → session.(N+1).jsonl, drop if > max_files-1
        for n, f in rotated:
            new_n = n + 1
            if new_n >= self.log_max_files:
                freed = self._delete_file(f, report)
                report.bytes_freed += freed
            else:
                new_path = parent / f"{stem}.{new_n}.jsonl"
                f.rename(new_path)

        # Rotate current file to .1
        new_path = parent / f"{stem}.1.jsonl"
        log_file.rename(new_path)
        # Recreate empty file for ongoing writes
        log_file.touch()

    # ------------------------------------------------------------------
    # Bulk operation
    # ------------------------------------------------------------------

    def run_all(self, active_profiles: set[str] | None = None) -> CompactionReport:
        """
        Run all compaction tasks. Non-fatal errors are collected, not raised.
        Returns a CompactionReport with totals.
        """
        report = CompactionReport()
        self.compact_playbooks(report)
        self.compact_cookies(report)
        self.compact_profiles(active_profiles, report)
        self.rotate_logs(report)
        log.info("DataLifecycleManager.run_all%s: %s", " [DRY]" if self.dry_run else "", report)
        return report

    # ------------------------------------------------------------------
    # Utility
    # ------------------------------------------------------------------

    @staticmethod
    def disk_usage(path: Path) -> int:
        """Recursively sum bytes under path. Returns 0 if path does not exist."""
        if not path.exists():
            return 0
        if path.is_file():
            return path.stat().st_size
        total = 0
        for root, _dirs, files in os.walk(path):
            for f in files:
                try:
                    total += Path(root, f).stat().st_size
                except OSError:
                    pass
        return total
