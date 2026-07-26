"""
Server-level tests: MCP protocol wiring and dispatch completeness.

These run without launching Chrome — they inspect the tool registry and drive
the JSON-RPC handler with dispatch mocked out.
"""
import inspect
import io
import json
import re
from contextlib import redirect_stdout
from unittest.mock import patch

from neobrowser import server


def _handled_tool_names() -> set:
    """Names the dispatch_tool if/elif chain handles, plus any plugin handlers."""
    src = inspect.getsource(server.dispatch_tool)
    handled = set(re.findall(r'name == "([a-z_]+)"', src))
    for grp in re.findall(r'name in \(([^)]*)\)', src):
        handled |= set(re.findall(r'"([a-z_]+)"', grp))
    handled |= set(server._PLUGIN_HANDLERS)
    return handled


def test_every_tool_has_a_dispatch_branch():
    missing = set(server.TOOLS) - _handled_tool_names()
    assert not missing, f"declared tools with no dispatch branch: {sorted(missing)}"


def test_no_orphan_dispatch_branches():
    # Every handled name should be a declared tool (guards against a typo'd
    # branch that can never be reached, or a tool renamed in TOOLS only).
    orphans = _handled_tool_names() - set(server.TOOLS)
    assert not orphans, f"dispatch branches with no matching tool: {sorted(orphans)}"


def test_tools_all_have_description_and_schema():
    for name, spec in server.TOOLS.items():
        assert spec.get("description"), f"{name} missing description"
        assert "schema" in spec, f"{name} missing schema"


def test_tools_list_returns_every_tool():
    buf = io.StringIO()
    with redirect_stdout(buf):
        server._handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})
    resp = json.loads(buf.getvalue().strip().splitlines()[-1])
    names = {t["name"] for t in resp["result"]["tools"]}
    assert names == set(server.TOOLS)


def test_initialize_responds():
    buf = io.StringIO()
    with redirect_stdout(buf):
        server._handle({"jsonrpc": "2.0", "id": 7, "method": "initialize", "params": {}})
    resp = json.loads(buf.getvalue().strip().splitlines()[-1])
    assert resp["id"] == 7
    assert "result" in resp


def test_unknown_tool_is_rejected():
    buf = io.StringIO()
    with redirect_stdout(buf):
        server._handle({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": {"name": "does_not_exist", "arguments": {}},
        })
    resp = json.loads(buf.getvalue().strip().splitlines()[-1])
    assert "error" in resp


def test_tools_call_dispatches_to_handler():
    # Drive a real tools/call but stub dispatch_tool so no Chrome launches.
    with patch.object(server, "dispatch_tool", return_value="ok-result") as md:
        buf = io.StringIO()
        with redirect_stdout(buf):
            server._handle({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "read", "arguments": {}},
            })
        md.assert_called_once_with("read", {})
    resp = json.loads(buf.getvalue().strip().splitlines()[-1])
    assert resp["result"]["content"][0]["text"] == "ok-result"
