"""
tools/v4/chrome_launcher.py

Python wrapper around chrome_launcher.sh.
Provides ChromeLauncher class for use in tests and the MCP server.

Usage:
    launcher = ChromeLauncher(port=9222)
    launcher.ensure()          # launch if not running
    launcher.is_running()      # health check
    launcher.stop()            # kill Chrome

Environment:
    NEOBROWSER_PORT  — override default port (default: 9222)
"""
from __future__ import annotations

import os
import subprocess
import time
import urllib.request
from pathlib import Path

CHROME_BIN = os.environ.get(
    "NEOBROWSER_CHROME_BIN",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
)
PROFILES_BASE = Path.home() / ".neorender" / "profiles"
PIDS_BASE = Path.home() / ".neorender"
DEFAULT_PORT = int(os.environ.get("NEOBROWSER_PORT", "9222"))


class ChromeLauncher:
    """
    Launch and health-check Chrome on a fixed port with a persistent profile.

    The profile dir (~/.neorender/profiles/{name}/) persists across restarts,
    preserving cookies, localStorage, and login sessions.
    """

    def __init__(self, port: int = DEFAULT_PORT, profile: str = "neorender") -> None:
        self.port = port
        self.profile = profile
        self._profile_dir = PROFILES_BASE / profile
        self._pidfile = PIDS_BASE / f"chrome-{port}.pid"

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def is_running(self) -> bool:
        """Return True if Chrome is responding on self.port."""
        try:
            urllib.request.urlopen(
                f"http://127.0.0.1:{self.port}/json/version", timeout=1.0
            )
            return True
        except Exception:
            return False

    def ensure(self, timeout_s: float = 10.0) -> None:
        """
        Start Chrome if not already running.  Blocks until ready or timeout.

        Raises RuntimeError if Chrome does not become ready within timeout_s.
        """
        if self.is_running():
            return

        self._profile_dir.mkdir(parents=True, exist_ok=True)
        PIDS_BASE.mkdir(parents=True, exist_ok=True)

        proc = subprocess.Popen(
            [
                CHROME_BIN,
                f"--remote-debugging-port={self.port}",
                f"--user-data-dir={self._profile_dir}",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-translate",
                "--disable-extensions",
                "--disable-default-apps",
                "--metrics-recording-only",
                "--safebrowsing-disable-auto-update",
                "--password-store=basic",
                "--use-mock-keychain",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._pidfile.write_text(str(proc.pid))

        deadline = time.monotonic() + timeout_s
        while time.monotonic() < deadline:
            if self.is_running():
                return
            time.sleep(0.25)

        proc.kill()
        self._pidfile.unlink(missing_ok=True)
        raise RuntimeError(
            f"Chrome did not become ready within {timeout_s}s "
            f"(port={self.port}, profile={self.profile})"
        )

    def stop(self) -> None:
        """Kill Chrome started by this launcher (via pidfile)."""
        if self._pidfile.exists():
            try:
                pid = int(self._pidfile.read_text().strip())
                os.kill(pid, 15)  # SIGTERM
                time.sleep(0.5)
                os.kill(pid, 9)   # SIGKILL fallback
            except (ProcessLookupError, ValueError):
                pass
            self._pidfile.unlink(missing_ok=True)

    def version(self) -> dict:
        """Return Chrome version info dict from /json/version."""
        import json
        resp = urllib.request.urlopen(
            f"http://127.0.0.1:{self.port}/json/version", timeout=2.0
        )
        return json.loads(resp.read())
