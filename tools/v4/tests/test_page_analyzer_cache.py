"""
tools/v4/tests/test_page_analyzer_cache.py

F08 — AX Snapshot Cache tests for PageAnalyzer.

All tests are pure unit tests: no Chrome, no network, no filesystem.
_fetch_snapshot is replaced with a mock that counts calls.
"""
from __future__ import annotations

import threading
import time
from unittest.mock import MagicMock

import pytest

from tools.v4.page_analyzer import PageAnalyzer


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_FAKE_NODES = [{"role": "button", "name": "Send", "nodeId": 1, "backendNodeId": 1}]


def _make_tab(tab_id: str = "t1", url: str = "https://example.com", nav_version: int = 0):
    tab = MagicMock()
    tab._tab_id = tab_id
    tab.current_url.return_value = url
    tab._nav_version = nav_version
    return tab


def _make_analyzer_with_mock(ttl: float = 5.0):
    analyzer = PageAnalyzer(cache_ttl_s=ttl)
    fetch_count = {"n": 0}

    def mock_fetch(tab):
        fetch_count["n"] += 1
        return list(_FAKE_NODES)

    analyzer._fetch_snapshot = mock_fetch
    return analyzer, fetch_count


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestAXSnapshotCache:

    def test_cache_hit_no_second_fetch(self):
        """Two snapshot() calls on same tab → only one CDP fetch."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab = _make_tab()

        analyzer.snapshot(tab)
        analyzer.snapshot(tab)

        assert fetch_count["n"] == 1

    def test_force_bypasses_cache(self):
        """snapshot(force=True) always fetches, even when cache is warm."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab = _make_tab()

        analyzer.snapshot(tab, force=True)
        analyzer.snapshot(tab, force=True)

        assert fetch_count["n"] == 2

    def test_cache_expires(self):
        """After TTL elapses, next call re-fetches from CDP."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=0.05)
        tab = _make_tab()

        analyzer.snapshot(tab)
        time.sleep(0.12)
        analyzer.snapshot(tab)

        assert fetch_count["n"] == 2

    def test_nav_version_change_invalidates(self):
        """Changing tab._nav_version between calls forces a new fetch."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab = _make_tab(nav_version=0)

        analyzer.snapshot(tab)
        tab._nav_version = 1  # simulate navigation
        analyzer.snapshot(tab)

        assert fetch_count["n"] == 2

    def test_invalidate_cache_clears_all(self):
        """invalidate_cache() causes next call to fetch fresh data."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab = _make_tab()

        analyzer.snapshot(tab)          # populates cache
        analyzer.invalidate_cache()
        analyzer.snapshot(tab)          # must re-fetch

        assert fetch_count["n"] == 2

    def test_invalidate_tab_only_removes_that_tab(self):
        """invalidate_tab(tab1) removes tab1 entries but leaves tab2 cached."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab1 = _make_tab(tab_id="t1")
        tab2 = _make_tab(tab_id="t2")

        analyzer.snapshot(tab1)        # cache tab1
        analyzer.snapshot(tab2)        # cache tab2
        assert fetch_count["n"] == 2

        analyzer.invalidate_tab(tab1)

        analyzer.snapshot(tab1)        # tab1 must re-fetch
        analyzer.snapshot(tab2)        # tab2 still cached

        assert fetch_count["n"] == 3

    def test_cache_ttl_zero_never_caches(self):
        """With ttl=0.0 every call fetches from CDP."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=0.0)
        tab = _make_tab()

        analyzer.snapshot(tab)
        analyzer.snapshot(tab)
        analyzer.snapshot(tab)

        assert fetch_count["n"] == 3

    def test_thread_safe_single_fetch(self):
        """5 concurrent threads calling snapshot() result in only 1 CDP fetch."""
        analyzer, fetch_count = _make_analyzer_with_mock(ttl=5.0)
        tab = _make_tab()

        # Add a small delay inside mock_fetch to maximise thread interleaving
        real_mock = analyzer._fetch_snapshot

        def slow_fetch(t):
            time.sleep(0.01)
            return real_mock(t)

        analyzer._fetch_snapshot = slow_fetch

        results = []
        errors = []

        def worker():
            try:
                snap = analyzer.snapshot(tab)
                results.append(snap)
            except Exception as exc:  # noqa: BLE001
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(5)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()

        assert not errors, f"Thread errors: {errors}"
        assert len(results) == 5
        # All threads should receive valid snapshots
        for snap in results:
            assert isinstance(snap, list)
            assert len(snap) > 0
        # Only 1 fetch (or very few — threads may race before first result stored)
        # The cache lock guarantees at most 1 wins the write, but multiple threads
        # may have called _fetch_snapshot before the first write completed.
        # The key invariant: no more than 5 fetches total (one per thread at worst).
        # In practice with the lock correctly protecting the read, it should be 1.
        assert fetch_count["n"] == 1
