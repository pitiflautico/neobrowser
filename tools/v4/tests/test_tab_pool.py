"""
tools/v4/tests/test_tab_pool.py

Unit tests for TabPool (F07).

All tests are offline — Session and ChromeTab are mocked.
"""
from __future__ import annotations

import threading
import time
from unittest.mock import MagicMock

import pytest

from tools.v4.tab_pool import TabPool


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_session(tabs=None):
    """Mock session whose open_tab() returns ChromeTab mocks in sequence."""
    session = MagicMock()
    tab_mocks = tabs or [MagicMock() for _ in range(5)]
    for t in tab_mocks:
        t.is_at.return_value = False
        t.close.return_value = None
    session.open_tab.side_effect = tab_mocks
    return session, tab_mocks


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestAcquireOpensNewTab:
    """1. First acquire() calls session.open_tab()."""

    def test_acquire_opens_new_tab_when_empty(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=3)

        acquired = pool.acquire()

        session.open_tab.assert_called_once()
        # The returned wrapper exposes the underlying tab
        assert acquired._tab is tabs[0]

        pool.release(acquired._tab)
        pool.close_all()


class TestAcquireReusesIdleTab:
    """2. Release then acquire → same tab returned; open_tab called only once."""

    def test_acquire_reuses_idle_tab(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=3)

        first = pool.acquire()
        pool.release(first._tab)
        second = pool.acquire()

        assert first._tab is second._tab
        session.open_tab.assert_called_once()

        pool.release(second._tab)
        pool.close_all()


class TestAcquireUrlMatchReusesTab:
    """3. tab.is_at(url)=True → that specific tab is returned without new open."""

    def test_acquire_url_match_reuses_tab(self):
        session, tabs = _make_session()
        target_url = "https://example.com/"
        # Make tabs[0] report it is at the target URL
        tabs[0].is_at.side_effect = lambda u: u == target_url

        pool = TabPool(session, size=3)

        # Seed the pool with one tab
        first = pool.acquire()           # opens tabs[0]
        pool.release(first._tab)         # tabs[0] back to idle

        second = pool.acquire(url=target_url)

        assert second._tab is tabs[0]
        session.open_tab.assert_called_once()   # no new tab opened

        pool.release(second._tab)
        pool.close_all()


class TestAcquireUrlNoMatchOpensNew:
    """4. No idle tab at url → opens a new tab."""

    def test_acquire_url_no_match_opens_new(self):
        session, tabs = _make_session()
        # All tabs return False for is_at (default in _make_session)

        pool = TabPool(session, size=3)

        first = pool.acquire()              # opens tabs[0]
        pool.release(first._tab)            # tabs[0] idle, but is_at returns False

        second = pool.acquire(url="https://other.com/")

        # tabs[0] is idle but is_at returns False; url-aware scan misses it.
        # Step 2 (any idle tab) should pick tabs[0] up since no URL match ran.
        # Wait — per spec: step 1 checks url match, step 2 returns first idle.
        # tabs[0] is idle, so step 2 picks it. No new tab should be opened.
        assert second._tab is tabs[0]
        session.open_tab.assert_called_once()

        pool.release(second._tab)
        pool.close_all()


class TestAcquireUrlNoMatchNewTabOpenedWhenNoIdle:
    """4b. No idle tab AND url no match → opens a new tab."""

    def test_acquire_url_no_match_opens_new_when_pool_not_full(self):
        session, tabs = _make_session()
        tabs[0].is_at.return_value = False

        pool = TabPool(session, size=3)

        first = pool.acquire()   # opens tabs[0], now in_use

        # Pool has 1 tab (in use), room for more → opens tabs[1]
        second = pool.acquire(url="https://other.com/")

        assert second._tab is tabs[1]
        assert session.open_tab.call_count == 2

        pool.release(first._tab)
        pool.release(second._tab)
        pool.close_all()


class TestAcquireBlocksWhenPoolFull:
    """5. size=1, acquire twice: second acquire blocks until release."""

    def test_acquire_blocks_when_pool_full(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=1, acquire_timeout_s=5.0)

        first = pool.acquire()   # occupies the only slot

        unblocked = threading.Event()
        second_tab_holder: list = []

        def _second():
            tab = pool.acquire()
            second_tab_holder.append(tab._tab)
            unblocked.set()

        t = threading.Thread(target=_second, daemon=True)
        t.start()

        # Thread should not unblock yet
        assert not unblocked.wait(timeout=0.2), "Second acquire should block"

        pool.release(first._tab)

        assert unblocked.wait(timeout=2.0), "Second acquire should unblock after release"
        assert second_tab_holder[0] is tabs[0]   # same tab reused

        pool.release(second_tab_holder[0])
        pool.close_all()


class TestAcquireTimeoutRaises:
    """6. size=1, pool full, acquire_timeout_s=0.1 → TimeoutError."""

    def test_acquire_timeout_raises(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=1, acquire_timeout_s=0.1)

        first = pool.acquire()  # fills the pool

        with pytest.raises(TimeoutError, match="TabPool"):
            pool.acquire()

        pool.release(first._tab)
        pool.close_all()


class TestReleaseNotifiesWaiter:
    """7. Thread 1 blocks in acquire(); thread 2 releases → thread 1 unblocks."""

    def test_release_notifies_waiting_acquire(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=1, acquire_timeout_s=5.0)

        first = pool.acquire()

        result: list = []
        error: list = []

        def _waiter():
            try:
                tab = pool.acquire()
                result.append(tab._tab)
            except Exception as e:
                error.append(e)

        t = threading.Thread(target=_waiter, daemon=True)
        t.start()

        time.sleep(0.15)   # give waiter time to block

        pool.release(first._tab)

        t.join(timeout=2.0)
        assert not error, f"Unexpected error: {error}"
        assert result, "Waiter did not receive a tab"
        assert result[0] is tabs[0]

        if result:
            pool.release(result[0])
        pool.close_all()


class TestCloseAllClosesEveryTab:
    """8. close_all() calls close() on every tab."""

    def test_close_all_closes_every_tab(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=3)

        t1 = pool.acquire()
        t2 = pool.acquire()
        pool.release(t1._tab)
        # t2 still in use, t1 idle

        pool.close_all()

        tabs[0].close.assert_called_once()
        tabs[1].close.assert_called_once()


class TestStatsCorrect:
    """9. Acquire 2 of 3 → stats = {total:2, idle:0, in_use:2, size:3}."""

    def test_stats_correct(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=3)

        t1 = pool.acquire()
        t2 = pool.acquire()

        s = pool.stats()
        assert s == {"total": 2, "idle": 0, "in_use": 2, "size": 3}

        pool.release(t1._tab)
        pool.release(t2._tab)
        pool.close_all()


class TestContextManagerReleasesOnExit:
    """10. `with pool.acquire() as tab:` releases on exit."""

    def test_context_manager_releases_on_exit(self):
        session, tabs = _make_session()
        pool = TabPool(session, size=3)

        with pool.acquire() as tab:
            assert tab is tabs[0]
            # Pool has 1 tab, 1 in use
            s = pool.stats()
            assert s["in_use"] == 1

        # After exit, tab should be idle
        s = pool.stats()
        assert s["in_use"] == 0
        assert s["idle"] == 1

        pool.close_all()
