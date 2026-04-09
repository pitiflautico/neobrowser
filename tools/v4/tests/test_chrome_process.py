"""
Unit tests for tools/v4/chrome_process.py

ALL tests run without launching real Chrome (mocks only).
"""
from __future__ import annotations

import io
import json
import os
import signal
import socket
from pathlib import Path
from unittest import mock
from unittest.mock import MagicMock, patch, call

import pytest

# ---------------------------------------------------------------------------
# Module under test
# ---------------------------------------------------------------------------
from tools.v4.chrome_process import (
    ChromeProcess,
    find_free_port,
    open_new_tab,
    wait_for_chrome,
)
import tools.v4.chrome_process as _cp_module


# ===========================================================================
# TestFindFreePort
# ===========================================================================
class TestFindFreePort:
    def test_returns_integer_in_valid_range(self):
        port = find_free_port()
        assert isinstance(port, int)
        assert 1024 <= port <= 65535


# ===========================================================================
# TestChromeProcessIsAlive
# ===========================================================================
class TestChromeProcessIsAlive:
    def _make(self, pid: int = 12345) -> ChromeProcess:
        return ChromeProcess(profile_dir=Path('/tmp/test'), port=9222, pid=pid)

    def test_alive_when_process_exists(self):
        cp = self._make()
        with patch('os.kill', return_value=None) as mock_kill:
            assert cp.is_alive() is True
            mock_kill.assert_called_once_with(cp.pid, 0)

    def test_dead_when_process_not_found(self):
        cp = self._make()
        with patch('os.kill', side_effect=ProcessLookupError):
            assert cp.is_alive() is False

    def test_dead_when_permission_error(self):
        """PermissionError means process exists but is not owned by us — treat as dead."""
        cp = self._make()
        with patch('os.kill', side_effect=PermissionError):
            assert cp.is_alive() is False


# ===========================================================================
# TestChromeProcessPortAlive
# ===========================================================================
class TestChromeProcessPortAlive:
    def _make(self, port: int = 9222) -> ChromeProcess:
        return ChromeProcess(profile_dir=Path('/tmp/test'), port=port, pid=99)

    def test_alive_when_http_responds(self):
        cp = self._make()
        fake_response = MagicMock()
        fake_response.status = 200
        fake_response.__enter__ = lambda s: s
        fake_response.__exit__ = MagicMock(return_value=False)

        with patch('urllib.request.urlopen', return_value=fake_response):
            assert cp.port_alive() is True

    def test_dead_when_connection_refused(self):
        cp = self._make()
        import urllib.error
        with patch('urllib.request.urlopen', side_effect=urllib.error.URLError('refused')):
            assert cp.port_alive() is False


# ===========================================================================
# TestChromeProcessHealthCheck
# ===========================================================================
class TestChromeProcessHealthCheck:
    def _make(self) -> ChromeProcess:
        return ChromeProcess(profile_dir=Path('/tmp/test'), port=9222, pid=99)

    def test_healthy_when_both_alive(self):
        cp = self._make()
        with patch.object(cp, 'is_alive', return_value=True), \
             patch.object(cp, 'port_alive', return_value=True):
            assert cp.health_check() is True

    def test_unhealthy_when_process_dead(self):
        cp = self._make()
        with patch.object(cp, 'is_alive', return_value=False), \
             patch.object(cp, 'port_alive', return_value=True):
            assert cp.health_check() is False

    def test_unhealthy_when_port_dead(self):
        cp = self._make()
        with patch.object(cp, 'is_alive', return_value=True), \
             patch.object(cp, 'port_alive', return_value=False):
            assert cp.health_check() is False


# ===========================================================================
# TestOpenNewTab
# ===========================================================================
class TestOpenNewTab:
    def _make_response(self, data: dict) -> MagicMock:
        body = json.dumps(data).encode()
        fake_resp = MagicMock()
        fake_resp.read.return_value = body
        fake_resp.__enter__ = lambda s: s
        fake_resp.__exit__ = MagicMock(return_value=False)
        return fake_resp

    def test_uses_put_method(self):
        """Capture the Request object sent to urlopen and verify method='PUT'."""
        captured = {}

        def fake_urlopen(req, timeout=None):
            captured['method'] = req.get_method()
            return self._make_response({'id': 'tab1'})

        with patch('urllib.request.urlopen', side_effect=fake_urlopen):
            open_new_tab(9222)

        assert captured['method'] == 'PUT'

    def test_url_contains_json_new(self):
        """Verify URL has /json/new."""
        captured = {}

        def fake_urlopen(req, timeout=None):
            captured['url'] = req.full_url
            return self._make_response({'id': 'tab1'})

        with patch('urllib.request.urlopen', side_effect=fake_urlopen):
            open_new_tab(9222)

        assert '/json/new' in captured['url']

    def test_returns_parsed_json(self):
        tab_data = {'id': 'abc123', 'type': 'page', 'url': 'about:blank'}

        def fake_urlopen(req, timeout=None):
            return self._make_response(tab_data)

        with patch('urllib.request.urlopen', side_effect=fake_urlopen):
            result = open_new_tab(9222)

        assert result == tab_data


# ===========================================================================
# TestWaitForChrome
# ===========================================================================
class TestWaitForChrome:
    def _make_ok_response(self) -> MagicMock:
        fake_resp = MagicMock()
        fake_resp.status = 200
        fake_resp.__enter__ = lambda s: s
        fake_resp.__exit__ = MagicMock(return_value=False)
        return fake_resp

    def test_returns_true_when_ready_immediately(self):
        with patch('urllib.request.urlopen', return_value=self._make_ok_response()), \
             patch('time.sleep'):
            assert wait_for_chrome(9222, timeout_s=5.0) is True

    def test_returns_false_on_timeout(self):
        """Mock always fails; with a tiny timeout it must return False quickly."""
        import urllib.error
        with patch('urllib.request.urlopen', side_effect=urllib.error.URLError('refused')), \
             patch('time.sleep'), \
             patch('time.monotonic', side_effect=[0.0, 0.05, 0.10, 0.15, 999.0]):
            # deadline = 0.0 + 0.01; second monotonic() call returns 0.05 > deadline
            assert wait_for_chrome(9222, timeout_s=0.01) is False

    def test_retries_until_ready(self):
        """Mock fails twice then succeeds; verify eventual True return."""
        import urllib.error
        call_count = {'n': 0}

        def side_effect(url, timeout=None):
            call_count['n'] += 1
            if call_count['n'] < 3:
                raise urllib.error.URLError('not ready')
            return self._make_ok_response()

        with patch('urllib.request.urlopen', side_effect=side_effect), \
             patch('time.sleep'), \
             patch('time.monotonic', return_value=0.0):
            result = wait_for_chrome(9222, timeout_s=30.0)

        assert result is True
        assert call_count['n'] == 3


# ===========================================================================
# TestChromeProcessLaunch
# ===========================================================================
class TestChromeProcessLaunch:
    @pytest.fixture(autouse=True)
    def _patch_profiles_base(self, tmp_path: Path):
        """Redirect PROFILES_BASE to tmp_path so launch() accepts tmp_path sub-dirs."""
        with patch.object(_cp_module, 'PROFILES_BASE', tmp_path):
            yield

    def test_launch_uses_correct_flags(self, tmp_path: Path):
        """Verify Popen is called with --remote-debugging-port, --headless=new, --user-data-dir."""
        mock_proc = MagicMock()
        mock_proc.pid = 54321
        launch_dir = tmp_path / 'profile'

        with patch('tools.v4.chrome_process.find_free_port', return_value=19222), \
             patch('subprocess.Popen', return_value=mock_proc) as mock_popen:
            ChromeProcess.launch(launch_dir)

        args = mock_popen.call_args[0][0]  # positional first arg = list
        joined = ' '.join(args)

        assert '--remote-debugging-port=19222' in joined
        assert '--headless=new' in joined
        assert f'--user-data-dir={launch_dir}' in joined

    def test_launch_returns_chrome_process_instance(self, tmp_path: Path):
        mock_proc = MagicMock()
        mock_proc.pid = 54321
        launch_dir = tmp_path / 'profile'

        with patch('tools.v4.chrome_process.find_free_port', return_value=19222), \
             patch('subprocess.Popen', return_value=mock_proc):
            cp = ChromeProcess.launch(launch_dir)

        assert isinstance(cp, ChromeProcess)
        assert cp.pid == 54321
        assert cp.port == 19222

    def test_launch_does_NOT_kill_external_pids(self, tmp_path: Path):
        """
        Critical regression test: launch() must NEVER call os.kill() on any pid.
        V3 bug: read a shared PID file and sent SIGTERM to whatever pid was in it,
        which could be a sibling process or unrelated service.
        """
        mock_proc = MagicMock()
        mock_proc.pid = 54321
        launch_dir = tmp_path / 'profile'

        with patch('tools.v4.chrome_process.find_free_port', return_value=19222), \
             patch('subprocess.Popen', return_value=mock_proc), \
             patch('os.kill') as mock_os_kill:
            ChromeProcess.launch(launch_dir)

        mock_os_kill.assert_not_called()


# ===========================================================================
# TestSecurityFixes
# ===========================================================================
class TestSecurityFixes:
    def test_pid_zero_raises(self, tmp_path: Path):
        with pytest.raises(ValueError, match="Refusing to manage PID"):
            ChromeProcess(tmp_path, 9222, 0)

    def test_pid_negative_raises(self, tmp_path: Path):
        with pytest.raises(ValueError, match="Refusing to manage PID"):
            ChromeProcess(tmp_path, 9222, -1)

    def test_invalid_port_raises_in_open_new_tab(self):
        with pytest.raises(ValueError, match="Invalid port"):
            open_new_tab(80)

    def test_invalid_port_raises_in_wait_for_chrome(self):
        with pytest.raises(ValueError, match="Invalid port"):
            wait_for_chrome(0)

    def test_profile_dir_outside_base_raises(self, tmp_path: Path):
        mock_proc = MagicMock()
        mock_proc.pid = 54321

        with patch('tools.v4.chrome_process.find_free_port', return_value=19222), \
             patch('subprocess.Popen', return_value=mock_proc):
            with pytest.raises(ValueError, match="profile_dir must be under"):
                ChromeProcess.launch(Path('/tmp/attack'))
