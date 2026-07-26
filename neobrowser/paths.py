"""
Central runtime-data locations for NeoBrowser.

Everything lives under NEOBROWSER_HOME (default ``~/.neobrowser``), overridable
via the ``NEOBROWSER_HOME`` environment variable. Keeping every path in one
module means the on-disk layout — profiles, cookies, sessions, playbooks, logs —
is defined in exactly one place.
"""
from __future__ import annotations

import os
from pathlib import Path


def _home() -> Path:
    env = os.environ.get("NEOBROWSER_HOME")
    return Path(env).expanduser() if env else Path.home() / ".neobrowser"


NEOBROWSER_HOME = _home()

PROFILES_BASE = NEOBROWSER_HOME / "profiles"    # ghost Chrome user-data dirs
COOKIES_BASE = NEOBROWSER_HOME / "cookies"      # per-profile JSON cookie snapshots
SESSIONS_BASE = NEOBROWSER_HOME / "sessions"    # full session caches (cookies + storage)
PLAYBOOKS_BASE = NEOBROWSER_HOME / "playbooks"  # recorded action playbooks
LOGS_BASE = NEOBROWSER_HOME / "logs"
PORT_FILE = NEOBROWSER_HOME / "neo-browser-port.txt"  # attach-mode port handoff
