"""
conftest.py — Operational test fixtures.

Loads neo-browser.py WITHOUT subprocess mocking so Chrome actually launches.
Keeps a single module instance across the session.

Important ordering note
-----------------------
pytest collects parent conftest.py files first.  tests/conftest.py patches
subprocess.Popen to block Chrome.  This conftest runs *after* that patch is
already applied to the subprocess module object.

We restore the real Popen before loading neo-browser by reaching back to the
original stored in the parent conftest's _real_popen, or (safer) by importing
the stdlib subprocess fresh from a new module loader.
"""
import sys
import importlib.util
import importlib
import subprocess
import pytest
from pathlib import Path

_NEO_PATH = Path(__file__).parent.parent.parent / 'tools' / 'v3' / 'neo-browser.py'

# ── Restore the real subprocess.Popen ──
# The parent tests/conftest.py saved the original as _real_popen in its own
# globals.  Grab it back; fall back to reimporting subprocess if needed.
def _restore_real_popen():
    # Try to retrieve it from the parent conftest module already in sys.modules
    for mod_name, mod in list(sys.modules.items()):
        rp = getattr(mod, '_real_popen', None)
        if rp is not None and callable(rp):
            subprocess.Popen = rp
            return rp
    # Fallback: reimport subprocess from scratch (builtins path, not cached)
    import builtins
    real_import = builtins.__import__
    _sp = real_import('subprocess', fromlist=[])
    # The patched Popen lives in the cached module; we need the C extension.
    # On CPython, the real Popen is available via subprocess.__init__ globals.
    # Safest: just use the saved reference that the parent conftest exported.
    return subprocess.Popen  # May still be patched; Chrome will fail gracefully

_real_popen = _restore_real_popen()


def _load_real_neo_browser():
    """Load neo-browser.py fresh with real subprocess AND real websockets."""
    # Remove stale module entries
    for key in ('neo_browser', 'neo_browser_real'):
        sys.modules.pop(key, None)

    # Remove websocket stubs injected by the parent conftest so that neo-browser
    # imports the real websockets package (installed in the env).
    for key in list(sys.modules):
        if key.startswith('websockets'):
            sys.modules.pop(key, None)

    # Ensure subprocess.Popen is real for Chrome launch
    subprocess.Popen = _real_popen

    spec = importlib.util.spec_from_file_location('neo_browser_real', str(_NEO_PATH))
    mod = importlib.util.module_from_spec(spec)
    sys.modules['neo_browser_real'] = mod
    spec.loader.exec_module(mod)

    # Keep real Popen in place permanently (Chrome calls happen later too)
    subprocess.Popen = _real_popen
    return mod


# Load once. If Chrome launches, it lives for the whole session.
try:
    _nb = sys.modules['neo_browser_real']
except KeyError:
    _nb = _load_real_neo_browser()


# ── Fixtures ──

@pytest.fixture(scope='session')
def nb():
    """The real neo_browser module."""
    return _nb


@pytest.fixture(scope='session')
def dispatch(nb):
    """Callable that invokes dispatch_tool on the real module."""
    def _dispatch(tool_name, args=None):
        return nb.dispatch_tool(tool_name, args or {})
    return _dispatch


@pytest.fixture(scope='session', autouse=True)
def kill_chrome_after_session(nb):
    """Kill Ghost Chrome when the session ends."""
    yield
    try:
        nb.cleanup()
    except Exception:
        pass
