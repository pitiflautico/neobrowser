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

import pytest

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


# ---------------------------------------------------------------------------
# Argument validation — a guessed parameter name must fail loudly, not be
# silently dropped in favour of the default.
# ---------------------------------------------------------------------------


def test_unknown_argument_is_rejected():
    with pytest.raises(ValueError) as exc:
        server.dispatch_tool("wait", {"seconds": 8})
    assert "seconds" in str(exc.value)
    assert "ms" in str(exc.value)  # the error names the parameter that does exist


def test_unknown_argument_rejected_before_plugin_handler_runs():
    server._PLUGIN_HANDLERS["fake_plugin"] = lambda args: "should not run"
    server.TOOLS["fake_plugin"] = {"description": "x", "schema": {"message": {"description": "m"}}}
    try:
        with pytest.raises(ValueError):
            server.dispatch_tool("fake_plugin", {"mesage": "typo"})
    finally:
        del server._PLUGIN_HANDLERS["fake_plugin"]
        del server.TOOLS["fake_plugin"]


def test_argument_error_is_reported_without_a_traceback():
    buf = io.StringIO()
    with redirect_stdout(buf):
        server._handle({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {"name": "wait", "arguments": {"seconds": 8}},
        })
    resp = json.loads(buf.getvalue().strip().splitlines()[-1])
    text = resp["result"]["content"][0]["text"]
    assert resp["result"]["isError"] is True
    assert "unknown argument(s): seconds" in text
    assert "Traceback" not in text


def test_missing_required_argument_is_rejected():
    with pytest.raises(ValueError) as exc:
        server.dispatch_tool("navigate", {})
    assert "url" in str(exc.value)


def test_valid_arguments_pass_validation():
    # Reaches the real handler (which then fails on no Chrome) — the point is
    # that validation itself did not raise.
    try:
        server._validate_args("wait", {"ms": 50, "selector": "body"})
        server._validate_args("navigate", {"url": "https://example.com", "wait_s": 1})
        server._validate_args("status", {})
    except ValueError as exc:
        raise AssertionError(f"valid arguments were rejected: {exc}")


def test_every_handler_only_reads_declared_parameters():
    """A parameter a handler honours but the schema omits is now unreachable.

    Validation rejects anything undeclared, so an undocumented-but-supported
    argument would break at the door. This keeps schema and handler in sync.
    """
    src = inspect.getsource(server.dispatch_tool)
    parts = re.split(r'\n    (?:el)?if name == "([a-z_]+)":', src)
    offenders = {}
    for i in range(1, len(parts), 2):
        tool, block = parts[i], parts[i + 1]
        used = set(re.findall(r'args(?:\.get\(|\[)["\']([a-zA-Z_]+)["\']', block))
        extra = used - set(server.TOOLS.get(tool, {}).get("schema", {}))
        if extra:
            offenders[tool] = sorted(extra)
    assert not offenders, f"handlers read undeclared parameters: {offenders}"
