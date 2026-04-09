"""
tools/v4/session.py

Tier 2: Session — manages one ChromeProcess per named profile.

Fixes V3 bugs:
- Zombie Chrome: ensure() health-checks before reusing, kills zombie and
  relaunches if dead. V3 reused a stale handle with no health check.
- Stale port file: Session is the authoritative source of the port.
  No ~/.neorender/neo-browser-port.txt shared global state.
- Cookie re-sync: set_cookies() / get_cookies() on ChromeTab can be called
  any time, not only once at startup (V3 bug).
- profile_name path traversal: sanitized before constructing profile_dir.
"""
from __future__ import annotations

import json
import re
import threading
from pathlib import Path

from tools.v4.chrome_process import PROFILES_BASE, ChromeProcess, wait_for_chrome
from tools.v4.chrome_tab import ChromeTab

# Allow alphanumeric, hyphens, underscores, and spaces (Chrome profile names use spaces).
_SAFE_NAME_RE = re.compile(r'^[a-zA-Z0-9][a-zA-Z0-9_\- ]{0,63}$')

CHROME_READY_TIMEOUT = 10.0  # seconds to wait for Chrome to start
COOKIES_BASE = Path.home() / ".neorender" / "cookies"


def _validate_cookie_path(path: Path) -> None:
    """Raise ValueError if path is not under COOKIES_BASE."""
    if not path.resolve().is_relative_to(COOKIES_BASE.resolve()):
        raise ValueError(f"Cookie path must be under {COOKIES_BASE}, got {path}")


def _validate_profile_name(name: str) -> None:
    """
    Reject profile names that could cause path traversal or filesystem issues.

    Allowed: letters, digits, hyphens, underscores. Max 64 chars.
    Starts with alphanumeric. No dots, slashes, null bytes, or spaces.
    """
    if not _SAFE_NAME_RE.match(name):
        raise ValueError(
            f"Invalid profile_name {name!r}. "
            "Only letters, digits, hyphens, and underscores allowed (max 64 chars)."
        )


class Session:
    """
    Manages a single headless Chrome process for a named browser profile.

    The profile directory is always PROFILES_BASE / profile_name.
    Session is the authoritative owner of the ChromeProcess — no shared
    PID files, no global port variables.

    Usage:
        session = Session("linkedin")
        tab = session.open_tab()
        tab.navigate("https://linkedin.com")
        tab.set_cookies([{"name": "...", "value": "...", "domain": "..."}])
        tab.close()
        session.close()

    Or as a context manager:
        with Session("linkedin") as session:
            with session.open_tab() as tab:
                tab.navigate("https://example.com")
    """

    def __init__(self, profile_name: str) -> None:
        _validate_profile_name(profile_name)
        self.profile_name = profile_name
        self._profile_dir: Path = PROFILES_BASE / profile_name
        self._chrome: ChromeProcess | None = None
        self._lock = threading.Lock()  # guards ensure() against concurrent launches

    # ------------------------------------------------------------------
    # Chrome lifecycle
    # ------------------------------------------------------------------

    def ensure(self) -> ChromeProcess:
        """
        Return a healthy ChromeProcess, launching one if needed.

        If the current chrome is alive and port-responsive, reuse it.
        If dead (zombie), kill it cleanly and launch a fresh one.

        V3 bug: chrome() returned a stale GhostChrome with no health check.
        """
        with self._lock:
            if self._chrome is not None and self._chrome.health_check():
                return self._chrome

            # Zombie or never started — kill if exists, then launch fresh.
            if self._chrome is not None:
                self._chrome.kill(force=True)
                self._chrome = None

            # Layer 1: sync real Chrome cookies to ghost profile dir BEFORE launch
            # so Chrome reads them natively at startup (SQLite + LocalStorage + IndexedDB)
            try:
                from tools.v4.cookie_sync import pre_launch_sync
                pre_launch_sync(self._profile_dir, self.profile_name)
            except Exception as exc:
                import logging
                logging.getLogger(__name__).warning(
                    "cookie_sync pre_launch_sync failed (non-fatal): %s", exc
                )

            chrome = ChromeProcess.launch(self._profile_dir)
            if not wait_for_chrome(chrome.port, timeout_s=CHROME_READY_TIMEOUT):
                chrome.kill(force=True)
                raise RuntimeError(
                    f"Chrome did not become ready within {CHROME_READY_TIMEOUT}s "
                    f"(port={chrome.port}, profile={self.profile_name})"
                )
            self._chrome = chrome
            return self._chrome

    def open_tab(self) -> ChromeTab:
        """
        Open a new tab in this session's Chrome instance.

        Calls ensure() to guarantee Chrome is healthy before opening.
        After opening, injects persisted cookies from JSON session cache
        so the tab starts authenticated without needing a navigation first.
        """
        chrome = self.ensure()
        tab = ChromeTab.open(chrome.port)
        # Layer 2: inject persisted cookies into the new tab via CDP
        try:
            from tools.v4.cookie_sync import post_launch_restore
            post_launch_restore(tab, self.profile_name)
        except Exception as exc:
            import logging
            logging.getLogger(__name__).debug(
                "cookie_sync post_launch_restore failed (non-fatal): %s", exc
            )
        return tab

    def close(self) -> None:
        """
        Terminate the Chrome process for this session.

        Safe to call multiple times. Does nothing if Chrome is not running.
        """
        if self._chrome is not None:
            self._chrome.kill(force=True)
            self._chrome = None

    # ------------------------------------------------------------------
    # Cookie persistence (F06)
    # ------------------------------------------------------------------

    def save_cookies(self, tab: ChromeTab, path: Path | None = None) -> None:
        """
        Save all cookies from tab to disk as JSON.
        Default path: ~/.neorender/cookies/{profile_name}.json
        File permissions: 0600 (owner read/write only).
        Does NOT log cookie values — only counts.
        """
        if path is None:
            path = COOKIES_BASE / f"{self.profile_name}.json"
        path = Path(path)
        _validate_cookie_path(path)
        COOKIES_BASE.mkdir(parents=True, exist_ok=True)
        cookies = tab.get_cookies()
        path.write_text(json.dumps(cookies, indent=2), encoding="utf-8")
        path.chmod(0o600)

    def restore_cookies(self, tab: ChromeTab, path: Path | None = None) -> int:
        """
        Load cookies from disk and inject into tab via Network.setCookies.
        Returns number of cookies restored.
        Returns 0 if file does not exist (no error).
        Returns 0 if file is corrupt JSON (logs warning, no error).
        """
        if path is None:
            path = COOKIES_BASE / f"{self.profile_name}.json"
        path = Path(path)
        _validate_cookie_path(path)
        if not path.exists():
            return 0
        try:
            cookies = json.loads(path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError) as exc:
            import logging
            logging.getLogger(__name__).warning("Failed to load cookies from %s: %s", path, exc)
            return 0
        if not cookies:
            return 0
        tab.set_cookies(cookies)
        return len(cookies)

    # ------------------------------------------------------------------
    # Context manager
    # ------------------------------------------------------------------

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    # ------------------------------------------------------------------
    # Repr
    # ------------------------------------------------------------------

    def __repr__(self) -> str:
        state = "running" if (self._chrome and self._chrome.health_check()) else "stopped"
        return f"Session(profile={self.profile_name!r}, chrome={state})"


class AttachedSession(Session):
    """
    Attaches to an already-running Chrome on a given CDP port.

    Does NOT launch or kill Chrome — the external process is not owned here.
    Ideal for agents running alongside a logged-in browser (e.g. LinkedIn,
    Gmail) where launching a new Chrome would lose the active session.

    Usage:
        session = AttachedSession(port=55715)
        tab = session.open_tab()
        tab.navigate("https://linkedin.com/messaging/")
        # session.close() is a no-op — Chrome keeps running

    Or via Browser.connect():
        with Browser.connect(55715) as b:
            tab = b.open("https://linkedin.com/messaging/")
            b.record_task("linkedin.com", "reply_message")
            ...
            b.stop_recording()
    """

    def __init__(self, port: int) -> None:
        # Bypass Session.__init__ — no profile dir, no ChromeProcess to manage.
        self.profile_name = f"attached-{port}"
        self._profile_dir = PROFILES_BASE / self.profile_name  # placeholder
        self._chrome = None
        self._lock = threading.Lock()
        self._port = port

    # ------------------------------------------------------------------
    # Override lifecycle — we don't own the Chrome process
    # ------------------------------------------------------------------

    def ensure(self):  # type: ignore[override]
        """No-op — Chrome is already running externally."""
        return None

    def open_tab(self) -> ChromeTab:
        """Open a new tab in the attached Chrome at self._port."""
        return ChromeTab.open(self._port)

    def close(self) -> None:
        """No-op — we don't kill Chrome we don't own."""

    def __repr__(self) -> str:
        return f"AttachedSession(port={self._port})"
