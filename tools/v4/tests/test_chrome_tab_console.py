"""
tests/test_chrome_tab_console.py

F01: Console Log Capture — unit tests for ChromeTab.enable_console(),
get_console_logs(), and clear_console_logs().
"""
from unittest.mock import MagicMock, call

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_tab() -> ChromeTab:
    ws = MagicMock()
    # recv() returns a minimal CDP response so send() doesn't block
    ws.recv.return_value = '{"id": 1, "result": {}}'
    tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
    # Reader thread is NOT started — we call _handle_page_event directly
    return tab


def _console_event(
    type_: str = "log",
    text: str = "hello",
    timestamp: float = 1234.0,
    url: str = "https://x.com",
) -> dict:
    return {
        "method": "Runtime.consoleAPICalled",
        "params": {
            "type": type_,
            "args": [{"type": "string", "value": text}],
            "timestamp": timestamp,
            "stackTrace": {"callFrames": [{"url": url}]},
        },
    }


def _exception_event(
    text: str = "Uncaught ReferenceError",
    desc: str = "ReferenceError: x is not defined",
) -> dict:
    return {
        "method": "Runtime.exceptionThrown",
        "params": {
            "timestamp": 9999.0,
            "exceptionDetails": {
                "text": text,
                "exception": {"description": desc},
            },
        },
    }


# ---------------------------------------------------------------------------
# Test 1: basic capture
# ---------------------------------------------------------------------------

def test_console_log_captured():
    tab = _make_tab()
    tab.enable_console()
    tab._handle_page_event(_console_event(type_="log", text="hello world", timestamp=100.0, url="https://example.com"))
    logs = tab.get_console_logs()
    assert len(logs) == 1
    entry = logs[0]
    assert entry["level"] == "log"
    assert entry["text"] == "hello world"
    assert entry["timestamp"] == 100.0
    assert entry["source"] == "https://example.com"


# ---------------------------------------------------------------------------
# Test 2: levels
# ---------------------------------------------------------------------------

def test_console_levels():
    tab = _make_tab()
    tab.enable_console()
    for level in ("warning", "error", "info"):
        tab._handle_page_event(_console_event(type_=level, text=f"{level} msg"))
    logs = tab.get_console_logs()
    assert len(logs) == 3
    found_levels = {e["level"] for e in logs}
    assert found_levels == {"warning", "error", "info"}


# ---------------------------------------------------------------------------
# Test 3: exception captured as error
# ---------------------------------------------------------------------------

def test_exception_captured_as_error():
    tab = _make_tab()
    tab.enable_console()
    tab._handle_page_event(_exception_event(text="Uncaught", desc="ReferenceError: x is not defined"))
    logs = tab.get_console_logs()
    assert len(logs) == 1
    entry = logs[0]
    assert entry["level"] == "error"
    assert entry["text"] == "ReferenceError: x is not defined"
    assert entry["timestamp"] == 9999.0
    assert entry["source"] == "exception"


# ---------------------------------------------------------------------------
# Test 4: not captured when disabled
# ---------------------------------------------------------------------------

def test_console_not_captured_when_disabled():
    tab = _make_tab()
    # enable_console() NOT called
    tab._handle_page_event(_console_event(type_="log", text="should not appear"))
    tab._handle_page_event(_exception_event())
    assert tab.get_console_logs() == []


# ---------------------------------------------------------------------------
# Test 5: clear
# ---------------------------------------------------------------------------

def test_clear_console_logs():
    tab = _make_tab()
    tab.enable_console()
    tab._handle_page_event(_console_event())
    tab._handle_page_event(_console_event())
    assert len(tab.get_console_logs()) == 2
    tab.clear_console_logs()
    assert tab.get_console_logs() == []


# ---------------------------------------------------------------------------
# Test 6: circular buffer cap at 500
# ---------------------------------------------------------------------------

def test_buffer_cap_500():
    tab = _make_tab()
    tab.enable_console()
    for i in range(501):
        tab._handle_page_event(_console_event(text=f"msg {i}", timestamp=float(i)))
    logs = tab.get_console_logs()
    assert len(logs) == 500
    # Oldest entry (msg 0) must have been dropped; newest (msg 500) must be present
    texts = [e["text"] for e in logs]
    assert "msg 0" not in texts
    assert "msg 500" in texts


# ---------------------------------------------------------------------------
# Test 7: get_console_logs returns a copy
# ---------------------------------------------------------------------------

def test_get_returns_snapshot_copy():
    tab = _make_tab()
    tab.enable_console()
    tab._handle_page_event(_console_event(text="original"))
    snapshot = tab.get_console_logs()
    # Mutate the returned list
    snapshot.append({"level": "log", "text": "injected", "timestamp": 0.0, "source": ""})
    snapshot[0]["text"] = "mutated"
    # Internal buffer must be unchanged
    internal = tab.get_console_logs()
    assert len(internal) == 1
    assert internal[0]["text"] == "original"


# ---------------------------------------------------------------------------
# Test 8: enable_console is idempotent — Runtime.enable sent only once
# ---------------------------------------------------------------------------

def test_enable_console_idempotent():
    ws = MagicMock()
    # Each send() needs a matching recv() response; keep a counter
    call_count = [0]

    def fake_recv(timeout=None):
        call_count[0] += 1
        return f'{{"id": {call_count[0]}, "result": {{}}}}'

    ws.recv.side_effect = fake_recv
    tab = ChromeTab(ws=ws, tab_id="t1", port=9222)

    tab.enable_console()
    tab.enable_console()
    tab.enable_console()

    # ws.send() should have been called exactly once with Runtime.enable
    runtime_enable_calls = [
        c for c in ws.send.call_args_list
        if '"Runtime.enable"' in c.args[0]
    ]
    assert len(runtime_enable_calls) == 1
