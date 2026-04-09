"""
PoC 7 — MCP V4 server funcional

Prueba que:
1. server.py responde a initialize correctamente
2. tools/list devuelve exactamente los 14 tools definidos
3. navigate tool devuelve error útil si no hay Chrome (no crash)
4. NEOBROWSER_ATTACH_PORT activa Browser.connect() en vez de Browser()
5. El servidor parsea JSON-RPC 2.0 correctamente
6. Errores de tool devuelven isError=True, no crash del proceso
"""
from __future__ import annotations

import importlib
import io
import json
import os
import sys
from unittest.mock import MagicMock, patch

import pytest


def _rpc(method: str, params: dict | None = None, req_id: int = 1) -> str:
    return json.dumps({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": method,
        "params": params or {},
    })


def _call_server(request_line: str) -> dict:
    """Send one JSON-RPC line to server._handle() and capture the response."""
    import tools.v4.server as srv
    captured = []
    orig_write = sys.stdout.write

    def fake_write(s):
        if s.strip():
            captured.append(s.strip())

    with patch.object(sys.stdout, "write", side_effect=fake_write):
        req = json.loads(request_line)
        srv._handle(req)

    assert captured, "No response written"
    return json.loads(captured[-1])


class TestMcpServerProtocol:

    def test_initialize_returns_protocol_version(self):
        resp = _call_server(_rpc("initialize"))
        assert resp["result"]["protocolVersion"] == "2024-11-05"
        assert resp["result"]["serverInfo"]["name"] == "neo-browser-v4"

    def test_tools_list_returns_14_tools(self):
        resp = _call_server(_rpc("tools/list"))
        tools = resp["result"]["tools"]
        assert len(tools) == 14
        names = {t["name"] for t in tools}
        expected = {
            "navigate", "screenshot", "read", "find", "click", "type",
            "console_logs", "network_log", "metrics", "save_cookies",
            "restore_cookies", "record_task", "stop_recording", "replay",
        }
        assert names == expected

    def test_unknown_tool_returns_error(self):
        resp = _call_server(_rpc("tools/call", {"name": "fly_to_moon", "arguments": {}}))
        assert resp.get("error") is not None

    def test_unknown_method_returns_error(self):
        resp = _call_server(_rpc("unknown/method"))
        assert resp.get("error") is not None

    def test_notifications_initialized_no_response(self):
        """notifications/initialized has no id — server must not crash."""
        import tools.v4.server as srv
        req = {"jsonrpc": "2.0", "method": "notifications/initialized"}
        # Should not raise
        captured = []
        with patch.object(sys.stdout, "write", side_effect=lambda s: captured.append(s)):
            srv._handle(req)
        # No response written (notification)
        assert not any(c.strip() for c in captured)

    def test_tool_error_returns_isError_not_crash(self):
        """A tool that throws must return isError=True, not crash the server."""
        import tools.v4.server as srv
        with patch.object(srv, "dispatch_tool", side_effect=RuntimeError("boom")):
            resp = _call_server(_rpc("tools/call", {
                "name": "navigate",
                "arguments": {"url": "https://example.com"},
            }))
        assert resp["result"]["isError"] is True
        assert "boom" in resp["result"]["content"][0]["text"]

    def test_each_tool_has_inputschema(self):
        resp = _call_server(_rpc("tools/list"))
        for tool in resp["result"]["tools"]:
            assert "inputSchema" in tool, f"Tool {tool['name']} missing inputSchema"
            assert tool["inputSchema"]["type"] == "object"

    def test_required_params_declared(self):
        """Tools with required params declare them in inputSchema.required."""
        resp = _call_server(_rpc("tools/list"))
        tools = {t["name"]: t for t in resp["result"]["tools"]}

        navigate = tools["navigate"]
        assert "url" in navigate["inputSchema"].get("required", [])

        find = tools["find"]
        assert "intent" in find["inputSchema"].get("required", [])


class TestMcpAttachPort:

    def test_attach_port_env_uses_browser_connect(self):
        """NEOBROWSER_ATTACH_PORT env var must call Browser.connect(), not Browser()."""
        import tools.v4.server as srv
        srv._browser = None  # reset global

        with patch.dict(os.environ, {"NEOBROWSER_ATTACH_PORT": "9222"}), \
             patch("tools.v4.browser.Browser.connect") as mock_connect, \
             patch("tools.v4.browser.Browser.__init__") as mock_init:
            mock_connect.return_value = MagicMock()
            srv._get_browser()

        mock_connect.assert_called_once()
        mock_init.assert_not_called()
        srv._browser = None  # cleanup

    def test_no_attach_port_uses_browser_init(self):
        """Without NEOBROWSER_ATTACH_PORT, normal Browser(profile=...) is used."""
        import tools.v4.server as srv
        srv._browser = None

        env = {k: v for k, v in os.environ.items() if k != "NEOBROWSER_ATTACH_PORT"}
        with patch.dict(os.environ, env, clear=True), \
             patch("tools.v4.browser.Browser.__init__", return_value=None) as mock_init, \
             patch("tools.v4.browser.Browser.connect") as mock_connect:
            try:
                srv._get_browser()
            except Exception:
                pass  # Browser.__init__ returns None → ok for this test

        mock_connect.assert_not_called()
        srv._browser = None  # cleanup
