"""
tools/v4/browser.py

F09 — Browser Facade

Single high-level object over Session + TabPool + PageAnalyzer.
Eliminates boilerplate: one import, one constructor, full browser.

Usage:
    with Browser(profile="linkedin") as b:
        tab = b.open("https://linkedin.com/messaging/")
        data = b.screenshot(tab)
        node_id = b.find(tab, "message input box")
        b.save_cookies(tab)
        b.close_tab(tab)
"""
from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

from tools.v4.session import Session, AttachedSession
from tools.v4.tab_pool import TabPool
from tools.v4.page_analyzer import PageAnalyzer
from tools.v4.playbook import ActionRecorder, PlaybookStore, PlaybookRunner, Step

if TYPE_CHECKING:
    from tools.v4.chrome_tab import ChromeTab


class Browser:
    """
    High-level facade over Session + TabPool + PageAnalyzer.

    One object → full browser: navigate, observe, interact, persist.

    Thread-safe: TabPool and PageAnalyzer are both thread-safe.
    Owns the Session lifecycle — call close() (or use as context manager)
    to shut down Chrome and release all tabs.
    """

    def __init__(
        self,
        profile: str = "default",
        pool_size: int = 3,
        ax_cache_ttl_s: float = 5.0,
    ) -> None:
        self._session = Session(profile)
        self._pool = TabPool(self._session, size=pool_size)
        self._analyzer = PageAnalyzer(cache_ttl_s=ax_cache_ttl_s)
        self.profile = profile
        self._recorder = ActionRecorder()
        self._store = PlaybookStore()
        self._runner = PlaybookRunner()
        self._recording_task: str | None = None
        self._recording_domain: str | None = None

    @classmethod
    def connect(
        cls,
        port: int,
        pool_size: int = 3,
        ax_cache_ttl_s: float = 5.0,
    ) -> "Browser":
        """
        Attach to an already-running Chrome on *port* without launching a new process.

        The caller owns the Chrome lifecycle — Browser.close() will NOT kill it.
        Use this when Chrome is already open with an active logged-in session
        (e.g. LinkedIn, Gmail) that would be lost if a new process were launched.

        Example::

            with Browser.connect(55715) as b:
                tab = b.open("https://linkedin.com/messaging/")
                b.record_task("linkedin.com", "reply_dm")
                node_id = b.find(tab, "message input box")
                tab.send("Input.insertText", {"text": "Hello from V4"})
                b.record_step(Step("type", {"text": "Hello from V4"}))
                b.stop_recording()
        """
        b = cls.__new__(cls)
        b._session = AttachedSession(port)
        b._pool = TabPool(b._session, size=pool_size)
        b._analyzer = PageAnalyzer(cache_ttl_s=ax_cache_ttl_s)
        b.profile = b._session.profile_name
        b._recorder = ActionRecorder()
        b._store = PlaybookStore()
        b._runner = PlaybookRunner()
        b._recording_task = None
        b._recording_domain = None
        return b

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def close(self) -> None:
        """Shut down all tabs and the Chrome process."""
        self._pool.close_all()
        self._session.close()

    def __enter__(self) -> "Browser":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    # ------------------------------------------------------------------
    # Navigation
    # ------------------------------------------------------------------

    def open(self, url: str, wait_s: float = 3.0) -> "ChromeTab":
        """
        Acquire a tab from the pool and navigate to url.

        If an idle tab is already at url, returns it without re-navigating.
        Call close_tab(tab) when done to return it to the pool.
        """
        tab = self._pool.acquire(url=url)
        # TabPool.acquire only does URL-aware reuse (checks tab.is_at(url)).
        # If the returned tab is not already at url, navigate now.
        if not tab.is_at(url):
            tab.navigate(url, wait_s=wait_s)
        return tab  # type: ignore[return-value]  # _AcquiredTab proxies ChromeTab

    def close_tab(self, tab: "ChromeTab") -> None:
        """Release tab back to the pool."""
        self._pool.release(tab)

    def attach_tab(self, tab_id: str) -> "ChromeTab":
        """
        Attach to an already-open Chrome tab by its target ID.

        Does NOT open a new tab or navigate. Returns a ChromeTab connected to
        the existing tab's WebSocket. The tab is NOT added to the pool — call
        tab.close() directly when done, or manage it outside the pool.

        Use this when you need to control a specific live tab (e.g. a logged-in
        LinkedIn conversation already open in the browser).

        Example::

            with Browser.connect(65315) as b:
                tab = b.attach_tab("B352EEF9A39C3F5E6154")
                print(tab.current_url())
                tab.close()
        """
        if not isinstance(self._session, AttachedSession):
            raise RuntimeError("attach_tab() requires Browser.connect(port) — not available in launch mode")
        import urllib.request, json as _json
        port = self._session._port
        resp = urllib.request.urlopen(f"http://localhost:{port}/json/list", timeout=3)
        tabs = _json.loads(resp.read())
        info = next((t for t in tabs if t.get("id") == tab_id), None)
        if info is None:
            raise ValueError(f"Tab {tab_id!r} not found in Chrome on port {port}")
        ws_url = info.get("webSocketDebuggerUrl", "")
        from tools.v4.chrome_tab import ChromeTab
        return ChromeTab.attach(ws_url, tab_id, port)

    # ------------------------------------------------------------------
    # Observability (delegate to tab)
    # ------------------------------------------------------------------

    def screenshot(self, tab: "ChromeTab", format: str = "png", quality: int = 80) -> bytes:
        """Capture the tab viewport as image bytes."""
        return tab.screenshot(format=format, quality=quality)

    def screenshot_save(
        self,
        tab: "ChromeTab",
        path: "str | Path",
        format: str = "png",
        quality: int = 80,
        base_dir: "str | Path | None" = None,
    ) -> Path:
        """Capture and save screenshot to disk. Returns resolved Path.
        Pass base_dir to enforce path containment when path is user-controlled."""
        return tab.screenshot_save(path=path, format=format, quality=quality, base_dir=base_dir)

    def console_logs(self, tab: "ChromeTab") -> list[dict]:
        """Return captured console log entries."""
        return tab.get_console_logs()

    def network_log(self, tab: "ChromeTab") -> list[dict]:
        """Return captured network requests."""
        return tab.get_network_requests()

    def metrics(self, tab: "ChromeTab") -> dict:
        """Return current performance metrics dict."""
        if not tab._performance_enabled:
            tab.enable_performance()
        return tab.get_metrics()

    # ------------------------------------------------------------------
    # Intelligence (PageAnalyzer with shared cache)
    # ------------------------------------------------------------------

    def snapshot(self, tab: "ChromeTab") -> list[dict]:
        """
        Return AX snapshot for the tab (cached by URL + nav version).
        Uses the shared PageAnalyzer instance with TTL cache.
        """
        return self._analyzer.snapshot(tab)

    def find(self, tab: "ChromeTab", intent: str) -> "int | None":
        """
        Find a UI element by intent description.
        Returns backendNodeId (int) or None if not found.
        Uses AX tree → heuristics → LLM fallback pipeline.
        """
        return self._analyzer.find_by_intent(tab, intent)

    # ------------------------------------------------------------------
    # Persistence (delegate to session)
    # ------------------------------------------------------------------

    def save_cookies(self, tab: "ChromeTab", path: "Path | None" = None) -> None:
        """Save all tab cookies to disk (~/.neorender/cookies/{profile}.json)."""
        self._session.save_cookies(tab, path=path)

    def restore_cookies(self, tab: "ChromeTab", path: "Path | None" = None) -> int:
        """Load cookies from disk and inject into tab. Returns count restored."""
        return self._session.restore_cookies(tab, path=path)

    def save_session(self, tab: "ChromeTab") -> dict:
        """
        Full session save: cookies + localStorage → ~/.neorender/sessions/{profile}/.
        Persists the authenticated state so future V4 startups restore it automatically.
        Returns stats dict: {cookies, domains, saved_at}.
        """
        from tools.v4.cookie_sync import save_session
        return save_session(tab, self.profile)

    def restore_local_storage(self, tab: "ChromeTab") -> int:
        """
        Inject saved localStorage into the current page via JS.
        Call after navigating to the target URL. Returns keys restored.
        """
        from tools.v4.cookie_sync import restore_local_storage
        return restore_local_storage(tab, self.profile)

    def session_info(self) -> dict:
        """Return session persistence state: manifest, file existence, sync TTL."""
        from tools.v4.cookie_sync import session_info
        return session_info(self.profile)

    # ------------------------------------------------------------------
    # Playbook (F10)
    # ------------------------------------------------------------------

    def record_task(self, domain: str, task_name: str) -> None:
        """Start recording steps for domain/task_name. Call record_step() after each action."""
        self._recording_domain = domain
        self._recording_task = task_name
        self._recorder.reset()

    def record_step(self, step: Step) -> None:
        """
        Append a Step to the active recording.

        Raises RuntimeError if record_task() was not called first.

        Example::

            b.record_step(Step("navigate", {"url": "https://linkedin.com/messaging/"}))
            b.record_step(Step("click_node", {"backend_node_id": 42, "role": "button", "name": "Send"},
                               fallback={"role": "button", "name": "Send"}))
            b.record_step(Step("type", {"text": "Hello"}))
        """
        if self._recording_domain is None:
            raise RuntimeError("No active recording. Call record_task() first.")
        self._recorder.record(step)

    def stop_recording(self) -> list[Step]:
        """Stop recording and save playbook. Returns recorded steps."""
        steps = self._recorder.get_playbook()
        if self._recording_domain and self._recording_task and steps:
            self._store.save(self._recording_domain, self._recording_task, steps)
        self._recording_domain = None
        self._recording_task = None
        return steps

    def replay(self, tab: "ChromeTab", domain: str, task_name: str) -> tuple[bool, int]:
        """
        Replay a saved playbook on tab. If no playbook exists, returns (False, -1).
        Returns (all_ok, first_failed_index) — first_failed_index=-1 if all steps ok.
        """
        steps = self._store.load(domain, task_name)
        if steps is None:
            return False, -1
        return self._runner.run(tab, steps, self._analyzer)

    # ------------------------------------------------------------------
    # Repr
    # ------------------------------------------------------------------

    def __repr__(self) -> str:
        stats = self._pool.stats()
        return (
            f"Browser(profile={self.profile!r}, "
            f"tabs={stats['total']}, "
            f"idle={stats['idle']}, "
            f"in_use={stats['in_use']})"
        )
