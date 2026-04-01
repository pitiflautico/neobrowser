"""
conftest.py — Safe import of neo-browser pure functions.

neo-browser.py has a hyphen in the filename, so we use importlib.
All Chrome/websocket/subprocess side-effects are suppressed before
the module executes.
"""
import sys
import types
import importlib.util
from pathlib import Path
from unittest.mock import MagicMock

import subprocess
import pytest

# ── 0. Prevent Chrome launch during tests ──
_real_popen = subprocess.Popen
def _fake_popen(*args, **kwargs):
    """Block any Chrome launch during tests."""
    cmd = args[0] if args else kwargs.get('args', [])
    if isinstance(cmd, (list, tuple)) and any('Chrome' in str(c) for c in cmd):
        mock = MagicMock()
        mock.pid = 99999
        mock.poll.return_value = 0
        mock.communicate.return_value = (b'', b'')
        return mock
    return _real_popen(*args, **kwargs)
subprocess.Popen = _fake_popen

# ── 1. Stub websockets before any import ──
_ws_mod = types.ModuleType('websockets')
_ws_sync_mod = types.ModuleType('websockets.sync')
_ws_sync_client_mod = types.ModuleType('websockets.sync.client')
_ws_sync_client_mod.connect = MagicMock(return_value=MagicMock())
_ws_mod.sync = _ws_sync_mod
_ws_sync_mod.client = _ws_sync_client_mod
sys.modules.setdefault('websockets', _ws_mod)
sys.modules.setdefault('websockets.sync', _ws_sync_mod)
sys.modules.setdefault('websockets.sync.client', _ws_sync_client_mod)

# ── 2. Load neo-browser via importlib (hyphen in filename) ──
_NEO_PATH = Path(__file__).parent.parent / 'tools' / 'v3' / 'neo-browser.py'
_PLUGINS_DIR = str(_NEO_PATH.parent)

if _PLUGINS_DIR not in sys.path:
    sys.path.insert(0, _PLUGINS_DIR)

def _load_neo_browser():
    spec = importlib.util.spec_from_file_location('neo_browser', str(_NEO_PATH))
    mod = importlib.util.module_from_spec(spec)
    sys.modules['neo_browser'] = mod
    spec.loader.exec_module(mod)
    return mod

# Load once at session start
try:
    nb = sys.modules['neo_browser']
except KeyError:
    nb = _load_neo_browser()


# ── Fixtures ──

@pytest.fixture(scope='session')
def neo():
    """The neo_browser module (loaded once per session)."""
    return nb


@pytest.fixture
def fresh_cache():
    """A fresh PageCache instance per test."""
    return nb.PageCache()
