"""
tools/v4/tab_pool.py

F07: TabPool — thread-safe pool of ChromeTab instances for a Session.

Avoids the overhead of opening a new tab on every task. Supports:
- URL-aware reuse: if an idle tab is already at the requested URL, return it
  without a fresh navigation.
- Blocking acquire with configurable timeout when all slots are in use.
- Context-manager support so callers can use `with pool.acquire() as tab:`.
"""
from __future__ import annotations

import threading
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tools.v4.session import Session
    from tools.v4.chrome_tab import ChromeTab


class _AcquiredTab:
    """
    Context-manager wrapper returned by TabPool.acquire().

    Releases the tab back to the pool on __exit__.
    Delegates attribute access to the underlying ChromeTab so callers can call
    tab methods directly: `with pool.acquire() as tab: tab.navigate(...)`.
    """

    def __init__(self, pool: "TabPool", tab: "ChromeTab") -> None:
        self._pool = pool
        self._tab = tab

    # Transparent proxy — pass through any attribute not defined here.
    def __getattr__(self, name: str):  # type: ignore[override]
        return getattr(self._tab, name)

    def __enter__(self) -> "ChromeTab":
        return self._tab

    def __exit__(self, *_: object) -> None:
        self._pool.release(self._tab)

    # Make the wrapper compare equal to the underlying tab so callers can do
    # `assert acquired is tab` or use it in sets.
    def __eq__(self, other: object) -> bool:
        if isinstance(other, _AcquiredTab):
            return self._tab is other._tab
        return self._tab is other

    def __hash__(self) -> int:
        # Must match the hash of the underlying ChromeTab so that
        # set membership checks (_in_use) work correctly.
        # hash(id(x)) != object.__hash__(x) for large addresses.
        return object.__hash__(self._tab)


class TabPool:
    """
    Thread-safe pool of ChromeTab instances managed by a single Session.

    Parameters
    ----------
    session:
        The Session that owns the Chrome process.  TabPool calls
        session.open_tab() to create new tabs.
    size:
        Maximum number of tabs the pool will hold simultaneously.
    acquire_timeout_s:
        How long (seconds) acquire() blocks when the pool is full and all
        tabs are in use before raising TimeoutError.

    Usage — basic::

        pool = TabPool(session, size=3)
        tab = pool.acquire()
        try:
            tab.navigate("https://example.com")
        finally:
            pool.release(tab)
        pool.close_all()

    Usage — context-manager (recommended)::

        with pool.acquire() as tab:
            tab.navigate("https://example.com")

    Usage — URL-aware reuse::

        with pool.acquire(url="https://example.com/") as tab:
            # If an idle tab is already at that URL it is returned as-is.
            # Otherwise a tab is returned (idle or new) and the caller is
            # responsible for navigating.
            ...
    """

    def __init__(
        self,
        session: "Session",
        size: int = 3,
        acquire_timeout_s: float = 10.0,
    ) -> None:
        self._session = session
        self._size = size
        self._acquire_timeout_s = acquire_timeout_s

        # Internal state — all guarded by _available (a Condition wrapping _lock).
        self._all: list["ChromeTab"] = []
        self._in_use: set["ChromeTab"] = set()

        self._lock = threading.Lock()
        self._available = threading.Condition(self._lock)

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def acquire(self, url: str | None = None) -> "_AcquiredTab":
        """
        Acquire a tab from the pool.

        Resolution order
        ----------------
        1. If *url* is given, scan idle tabs for one where ``tab.is_at(url)``
           returns True — return it immediately without a new navigation.
        2. Return the first idle tab (any URL).
        3. If the pool has not reached *size*, open a new tab via
           ``session.open_tab()`` and add it to the pool.
        4. Block on a Condition until a tab becomes available or the timeout
           expires; raise ``TimeoutError`` on timeout.

        Returns an ``_AcquiredTab`` context-manager wrapper that releases the
        tab when used as a ``with`` statement.
        """
        with self._available:
            while True:
                # --- Step 1: URL-aware reuse ---
                if url is not None:
                    for tab in self._all:
                        if tab not in self._in_use and tab.is_at(url):
                            self._in_use.add(tab)
                            return _AcquiredTab(self, tab)

                # --- Step 2: any idle tab ---
                idle = [t for t in self._all if t not in self._in_use]
                if idle:
                    tab = idle[0]
                    self._in_use.add(tab)
                    return _AcquiredTab(self, tab)

                # --- Step 3: pool has room — open a new tab ---
                if len(self._all) < self._size:
                    # Release the lock while doing I/O so other threads can
                    # call release() while we're waiting for Chrome.
                    # We use wait(0) to release, then re-check the condition.
                    # Simpler: just open the tab under the lock — open_tab()
                    # is fast (one HTTP PUT to localhost).
                    tab = self._session.open_tab()
                    self._all.append(tab)
                    self._in_use.add(tab)
                    return _AcquiredTab(self, tab)

                # --- Step 4: pool full, block ---
                # wait() releases _lock, sleeps, then reacquires it.
                got = self._available.wait(timeout=self._acquire_timeout_s)
                if not got:
                    # No notification arrived within the timeout.
                    # Do one last scan before giving up (a release might have
                    # arrived between the wait() expiry and the lock re-acquire).
                    idle = [t for t in self._all if t not in self._in_use]
                    if idle:
                        tab = idle[0]
                        self._in_use.add(tab)
                        return _AcquiredTab(self, tab)
                    raise TimeoutError(
                        f"TabPool: no tab available after {self._acquire_timeout_s}s "
                        f"(size={self._size}, in_use={len(self._in_use)})"
                    )
                # A notification was received — loop and re-evaluate.

    def release(self, tab: "ChromeTab") -> None:
        """
        Return a tab to the idle pool.

        Thread-safe.  Silently ignores tabs not owned by this pool.
        Notifies all threads waiting in acquire().
        """
        with self._available:
            if tab not in self._in_use:
                return  # unknown or already released — ignore
            self._in_use.discard(tab)
            self._available.notify_all()

    def close_all(self) -> None:
        """
        Close every tab owned by this pool (idle and in-use).

        Clears all internal state.  After this call the pool is empty and
        can be reused (acquire() will open new tabs on demand).
        """
        with self._available:
            tabs = list(self._all)
            self._all.clear()
            self._in_use.clear()

        # Close tabs outside the lock to avoid holding it during I/O.
        for tab in tabs:
            try:
                tab.close()
            except Exception:
                pass

        # Notify any waiters so they can unblock and get an error or new tab.
        with self._available:
            self._available.notify_all()

    def stats(self) -> dict:
        """
        Return a snapshot of pool utilisation.

        Returns
        -------
        dict with keys:
        - ``"total"``  — number of tabs owned by the pool
        - ``"idle"``   — number of tabs not currently in use
        - ``"in_use"`` — number of tabs currently acquired
        - ``"size"``   — configured pool capacity
        """
        with self._lock:
            total = len(self._all)
            in_use = len(self._in_use)
            return {
                "total": total,
                "idle": total - in_use,
                "in_use": in_use,
                "size": self._size,
            }
