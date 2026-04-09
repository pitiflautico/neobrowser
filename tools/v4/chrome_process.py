"""
tools/v4/chrome_process.py

Tier 0: Clean Chrome process manager.

Fixes V3 bugs:
- No shared PID file that kills sibling processes
- ChromeProcess only kills self.pid (the pid it launched)
- health_check() prevents zombie GhostChrome
- open_new_tab() uses PUT (V3 used GET → 405)
- No code runs at import time
"""
from __future__ import annotations

import os
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request
import json
from pathlib import Path

PROFILES_BASE = Path.home() / '.neorender' / 'profiles'

CHROME_BIN = os.environ.get(
    'NEOBROWSER_CHROME_BIN',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
)
CHROME_UA = (
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) '
    'AppleWebKit/537.36 (KHTML, like Gecko) '
    'Chrome/124.0.0.0 Safari/537.36'
)

DEFAULT_CHROME_FLAGS = [
    '--headless=new',
    '--no-sandbox',
    '--disable-gpu',
    '--disable-dev-shm-usage',
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-background-networking',
    '--disable-sync',
    '--disable-translate',
    '--mute-audio',
    f'--user-agent={CHROME_UA}',
]


def _validate_port(port: int) -> None:
    if not (1024 <= port <= 65535):
        raise ValueError(f"Invalid port {port}")


def find_free_port() -> int:
    """Find a free TCP port by binding to port 0 and letting the OS assign one."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def wait_for_chrome(port: int, timeout_s: float = 10.0) -> bool:
    """
    Poll GET /json/version until Chrome responds or timeout expires.

    Returns True if Chrome became ready within timeout_s, False otherwise.
    """
    _validate_port(port)
    url = f'http://127.0.0.1:{port}/json/version'
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=1.0) as resp:
                if resp.status == 200:
                    return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def open_new_tab(port: int) -> dict:
    """
    Open a new tab via Chrome DevTools Protocol.

    IMPORTANT: Must use PUT not GET. GET returns HTTP 405.
    V3 bug: used GET → always got 405.
    """
    _validate_port(port)
    url = f'http://127.0.0.1:{port}/json/new'
    req = urllib.request.Request(url, method='PUT')
    with urllib.request.urlopen(req, timeout=5.0) as resp:
        return json.loads(resp.read().decode())


class ChromeProcess:
    """
    Manages a single headless Chrome process.

    Owns exactly one PID. kill() only ever sends signals to self.pid.
    No shared PID files, no risk of killing sibling processes.
    """

    def __init__(self, profile_dir: Path, port: int, pid: int):
        if pid <= 1:
            raise ValueError(f"Refusing to manage PID {pid}")
        self.profile_dir = profile_dir
        self.port = port
        self.pid = pid

    @classmethod
    def launch(
        cls,
        profile_dir: Path,
    ) -> 'ChromeProcess':
        """
        Launch headless Chrome on a free port.

        Does NOT read or write any shared PID file.
        Does NOT kill any existing process.
        Returns a ChromeProcess bound to the spawned PID.
        profile_dir must be under PROFILES_BASE (~/.neorender/profiles/).
        """
        port = find_free_port()
        profile_dir = Path(profile_dir)
        if not profile_dir.resolve().is_relative_to(PROFILES_BASE.resolve()):
            raise ValueError(f"profile_dir must be under {PROFILES_BASE}")
        profile_dir.mkdir(parents=True, exist_ok=True)

        flags = [
            CHROME_BIN,
            f'--remote-debugging-port={port}',
            f'--user-data-dir={profile_dir}',
        ] + DEFAULT_CHROME_FLAGS

        proc = subprocess.Popen(
            flags,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
        )
        return cls(profile_dir=profile_dir, port=port, pid=proc.pid)

    def is_alive(self) -> bool:
        """
        Check if the Chrome process is still running.

        Uses os.kill(pid, 0) — sends no signal, just checks existence.
        Returns False if process is gone or not owned by this user.
        """
        try:
            os.kill(self.pid, 0)
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            # Process exists but belongs to another user — not ours.
            return False

    def port_alive(self) -> bool:
        """Check if Chrome's HTTP debug endpoint responds."""
        _validate_port(self.port)
        url = f'http://127.0.0.1:{self.port}/json/version'
        try:
            with urllib.request.urlopen(url, timeout=2.0) as resp:
                return resp.status == 200
        except Exception:
            return False

    def health_check(self) -> bool:
        """
        Returns True only if BOTH the process is alive AND the port responds.

        V3 bug: chrome() returned a zombie GhostChrome without checking.
        This method prevents that: callers can verify before using.
        """
        return self.is_alive() and self.port_alive()

    def kill(self, force: bool = False) -> None:
        """
        Terminate this Chrome process.

        Sends SIGTERM first. If force=True, waits up to 3 seconds then SIGKILL.
        Only ever touches self.pid — never touches external PIDs.
        """
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            return  # Already gone, nothing to do.

        if force:
            deadline = time.monotonic() + 3.0
            while time.monotonic() < deadline:
                if not self.is_alive():
                    return
                time.sleep(0.1)
            # Still alive after 3s — escalate to SIGKILL.
            try:
                os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
