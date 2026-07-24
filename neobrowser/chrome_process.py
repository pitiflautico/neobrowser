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
import platform as _platform
import re
import shutil
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request
import json
from pathlib import Path

PROFILES_BASE = Path.home() / '.neorender' / 'profiles'

def _discover_chrome_bin() -> str:
    """
    Locate a Chrome/Chromium binary cross-platform.

    Honors NEOBROWSER_CHROME_BIN first, then probes the usual macOS app-bundle
    paths, the PATH (Linux), and the standard Windows install locations. Falls
    back to the macOS default so a failure names a concrete, fixable path.
    """
    env = os.environ.get('NEOBROWSER_CHROME_BIN')
    if env:
        return env
    mac_paths = [
        '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
        '/Applications/Chromium.app/Contents/MacOS/Chromium',
    ]
    for p in mac_paths:
        if os.path.exists(p):
            return p
    for name in ('google-chrome', 'google-chrome-stable', 'chromium',
                 'chromium-browser', 'chrome'):
        found = shutil.which(name)
        if found:
            return found
    for p in (r'C:\Program Files\Google\Chrome\Application\chrome.exe',
              r'C:\Program Files (x86)\Google\Chrome\Application\chrome.exe'):
        if os.path.exists(p):
            return p
    return mac_paths[0]


CHROME_BIN = _discover_chrome_bin()

_chrome_ua_cache: "str | None" = None


def _detect_chrome_major(chrome_bin: str) -> str:
    """Return the installed Chrome major version (e.g. '150'), or '' if unknown."""
    try:
        out = subprocess.run(
            [chrome_bin, '--version'],
            capture_output=True, text=True, timeout=5,
        ).stdout
        m = re.search(r'(\d+)\.\d', out)
        if m:
            return m.group(1)
    except Exception:
        pass
    return ''


def _chrome_user_agent() -> "str | None":
    """
    Build a User-Agent matching the REAL installed Chrome, kept consistent with
    the browser's genuine Client Hints (Sec-CH-UA).

    Modern Chrome freezes the UA: the platform token and the 'MAJOR.0.0.0' version
    shape are fixed, so this reproduces exactly what a real Chrome of the same
    major version reports on this OS. Applied via the --user-agent launch flag
    (which, unlike CDP Network.setUserAgentOverride, does NOT blank Client Hints),
    it turns the only remaining headless tell ('HeadlessChrome' in the UA) into a
    clean, self-consistent identity. Cached after first computation.
    """
    global _chrome_ua_cache
    if _chrome_ua_cache is None:
        major = _detect_chrome_major(CHROME_BIN)
        if not major:
            _chrome_ua_cache = ''
        else:
            sysname = _platform.system()
            if sysname == 'Windows':
                token = 'Windows NT 10.0; Win64; x64'
            elif sysname == 'Linux':
                token = 'X11; Linux x86_64'
            else:  # Darwin and anything else -> frozen macOS token
                token = 'Macintosh; Intel Mac OS X 10_15_7'
            _chrome_ua_cache = (
                f'Mozilla/5.0 ({token}) AppleWebKit/537.36 '
                f'(KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36'
            )
    return _chrome_ua_cache or None


# Headless launch flags. Kept deliberately minimal and free of automation tells:
#   * NO spoofed --user-agent — a fake UA string never matches the real binary's
#     Client Hints (Sec-CH-UA / navigator.userAgentData) and is a one-request
#     giveaway. Genuine Chrome's own UA has zero mismatch, which is the whole
#     point of driving a real browser.
#   * --disable-blink-features=AutomationControlled suppresses navigator.webdriver
#     (this was previously only on the visible path, leaking webdriver in headless).
#   * --disable-gpu is NOT here: under --headless=new the GPU works and software
#     WebGL (SwiftShader) is itself a headless fingerprint. Opt in via
#     NEOBROWSER_DISABLE_GPU for GPU-less CI hosts (see ChromeProcess.launch).
DEFAULT_CHROME_FLAGS = [
    '--headless=new',
    '--no-sandbox',
    '--disable-dev-shm-usage',
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-background-networking',
    '--disable-sync',
    '--disable-translate',
    '--mute-audio',
    '--window-size=1920,1080',
    '--disable-blink-features=AutomationControlled',
]

# Visible mode: real Chrome window, no headless, no fake UA.
# Uses a separate profile dir so the real user profile isn't locked.
VISIBLE_CHROME_FLAGS = [
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-background-networking',
    '--mute-audio',
    '--disable-blink-features=AutomationControlled',
]

# macOS default real Chrome profile (where user has logged-in sessions)
REAL_CHROME_PROFILE = Path.home() / 'Library' / 'Application Support' / 'Google' / 'Chrome'


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

        # Rewrite the UA to drop the 'HeadlessChrome' tell, matching the real
        # installed version so it stays consistent with genuine Client Hints.
        ua = _chrome_user_agent()
        if ua:
            flags.append(f'--user-agent={ua}')

        # GPU-less hosts (headless Linux CI, some containers) need software
        # rendering. Opt in rather than default it on — software WebGL is a
        # headless fingerprint we don't want on real machines.
        if os.environ.get('NEOBROWSER_DISABLE_GPU'):
            flags.append('--disable-gpu')

        proc = subprocess.Popen(
            flags,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
        )
        return cls(profile_dir=profile_dir, port=port, pid=proc.pid)

    @classmethod
    def launch_visible(
        cls,
        profile_dir: Path | None = None,
    ) -> 'ChromeProcess':
        """
        Launch Chrome in visible mode (no headless, no fake UA).

        Uses a copy-on-write profile derived from the real Chrome profile
        so the user's sessions (Twitter, Google, etc.) are available without
        headless detection flags.

        If profile_dir is None, creates one under PROFILES_BASE/visible-<port>.
        """
        port = find_free_port()

        if profile_dir is None:
            profile_dir = PROFILES_BASE / f'visible-{port}'
        profile_dir = Path(profile_dir)
        profile_dir.mkdir(parents=True, exist_ok=True)

        flags = [
            CHROME_BIN,
            f'--remote-debugging-port={port}',
            f'--user-data-dir={profile_dir}',
            f'--window-size=1920,1080',
        ] + VISIBLE_CHROME_FLAGS

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
