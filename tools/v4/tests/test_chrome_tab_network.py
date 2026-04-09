"""
tools/v4/tests/test_chrome_tab_network.py

Unit tests for F02 — Network Trace on ChromeTab.
All tests use a mock WebSocket — no Chrome process required.
"""
from __future__ import annotations

import copy
import json
import queue
import threading
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Mock WebSocket helper
# ---------------------------------------------------------------------------


class MockWS:
    """Minimal WebSocket mock that accepts send() and has a recv queue."""

    def __init__(self):
        self._send_queue: queue.Queue = queue.Queue()
        self._recv_queue: queue.Queue = queue.Queue()
        self.sent: list[dict] = []

    def send(self, payload: str) -> None:
        msg = json.loads(payload)
        self.sent.append(msg)
        # Auto-respond to any command with an empty result
        response = json.dumps({"id": msg["id"], "result": {}})
        self._recv_queue.put_nowait(response)

    def recv(self, timeout: float = 30.0) -> str:
        try:
            return self._recv_queue.get(timeout=timeout)
        except queue.Empty:
            raise TimeoutError("mock recv timeout")

    def inject_event(self, event: dict) -> None:
        """Inject a CDP event for the reader/send to pick up."""
        self._recv_queue.put_nowait(json.dumps(event))

    def close(self) -> None:
        pass


def make_tab() -> ChromeTab:
    """Create a ChromeTab with a MockWS — reader thread NOT started."""
    ws = MockWS()
    tab = ChromeTab(ws=ws, tab_id="test-tab-id", port=9222)
    return tab


# ---------------------------------------------------------------------------
# Helper: feed events directly into _handle_page_event
# ---------------------------------------------------------------------------


def send_event(tab: ChromeTab, event: dict) -> None:
    tab._handle_page_event(event)


# ---------------------------------------------------------------------------
# Test 1: enable_network() sets _network_enabled=True
# ---------------------------------------------------------------------------


def test_enable_network_sets_flag():
    tab = make_tab()
    assert tab._network_enabled is False
    tab.enable_network()
    assert tab._network_enabled is True


# ---------------------------------------------------------------------------
# Test 2: enable_network() is idempotent
# ---------------------------------------------------------------------------


def test_enable_network_idempotent():
    tab = make_tab()
    tab.enable_network()
    tab.enable_network()  # second call must not raise
    assert tab._network_enabled is True
    # Only one Network.enable command should have been sent
    network_enables = [m for m in tab._ws.sent if m["method"] == "Network.enable"]
    assert len(network_enables) == 1


# ---------------------------------------------------------------------------
# Test 3: requestWillBeSent populates entry
# ---------------------------------------------------------------------------


def test_request_will_be_sent_populates_entry():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-1",
            "request": {"url": "https://example.com/", "method": "GET"},
            "timestamp": 1000.0,
        },
    })

    requests = tab.get_network_requests()
    assert len(requests) == 1
    r = requests[0]
    assert r["request_id"] == "req-1"
    assert r["url"] == "https://example.com/"
    assert r["method"] == "GET"
    assert r["timestamp"] == 1000.0
    assert r["status"] is None
    assert r["error"] is None


# ---------------------------------------------------------------------------
# Test 4: responseReceived updates status/duration/size
# ---------------------------------------------------------------------------


def test_response_received_updates_entry():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-1",
            "request": {"url": "https://example.com/", "method": "GET"},
            "timestamp": 1000.0,
        },
    })
    send_event(tab, {
        "method": "Network.responseReceived",
        "params": {
            "requestId": "req-1",
            "timestamp": 1000.5,
            "response": {
                "status": 200,
                "statusText": "OK",
                "encodedDataLength": 1234,
            },
        },
    })

    r = tab.get_network_requests()[0]
    assert r["status"] == 200
    assert r["status_text"] == "OK"
    assert r["encoded_data_length"] == 1234
    assert r["duration_ms"] == pytest.approx(500.0, abs=1.0)


# ---------------------------------------------------------------------------
# Test 5: loadingFailed sets error field
# ---------------------------------------------------------------------------


def test_loading_failed_sets_error():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-err",
            "request": {"url": "https://notexist.invalid/", "method": "GET"},
            "timestamp": 1.0,
        },
    })
    send_event(tab, {
        "method": "Network.loadingFailed",
        "params": {
            "requestId": "req-err",
            "errorText": "net::ERR_NAME_NOT_RESOLVED",
        },
    })

    r = tab.get_network_request("notexist.invalid")
    assert r is not None
    assert r["error"] == "net::ERR_NAME_NOT_RESOLVED"


# ---------------------------------------------------------------------------
# Test 6: get_network_requests() returns deep copy
# ---------------------------------------------------------------------------


def test_get_network_requests_returns_deep_copy():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-1",
            "request": {"url": "https://example.com/", "method": "GET"},
            "timestamp": 1.0,
        },
    })

    result = tab.get_network_requests()
    result[0]["url"] = "https://MUTATED.com/"  # mutate returned copy

    # Internal state unchanged
    internal = tab.get_network_requests()
    assert internal[0]["url"] == "https://example.com/"


# ---------------------------------------------------------------------------
# Test 7: get_network_request() finds matching URL, returns None on no match
# ---------------------------------------------------------------------------


def test_get_network_request_find_and_none():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-1",
            "request": {"url": "https://example.com/page", "method": "GET"},
            "timestamp": 1.0,
        },
    })

    found = tab.get_network_request("example.com")
    assert found is not None
    assert "example.com" in found["url"]

    not_found = tab.get_network_request("notinthebuffer.xyz")
    assert not_found is None


# ---------------------------------------------------------------------------
# Test 8: clear_network_log() empties buffer, enable still active
# ---------------------------------------------------------------------------


def test_clear_network_log():
    tab = make_tab()
    tab.enable_network()

    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-1",
            "request": {"url": "https://example.com/", "method": "GET"},
            "timestamp": 1.0,
        },
    })

    assert len(tab.get_network_requests()) == 1
    tab.clear_network_log()
    assert len(tab.get_network_requests()) == 0
    assert tab._network_enabled is True  # still enabled


# ---------------------------------------------------------------------------
# Test 9: buffer capped at 200 (oldest evicted)
# ---------------------------------------------------------------------------


def test_buffer_capped_at_200():
    tab = make_tab()
    tab.enable_network()

    for i in range(250):
        send_event(tab, {
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": f"req-{i}",
                "request": {"url": f"https://example.com/page{i}", "method": "GET"},
                "timestamp": float(i),
            },
        })

    requests = tab.get_network_requests()
    assert len(requests) == 200
    # req-0 through req-49 should be evicted (oldest)
    ids = {r["request_id"] for r in requests}
    assert "req-0" not in ids
    assert "req-249" in ids


# ---------------------------------------------------------------------------
# Test 10: get_network_requests() without enable returns []
# ---------------------------------------------------------------------------


def test_get_network_requests_without_enable_returns_empty():
    tab = make_tab()
    # Do NOT call enable_network()
    # Inject event — should be ignored because not enabled
    send_event(tab, {
        "method": "Network.requestWillBeSent",
        "params": {
            "requestId": "req-ignored",
            "request": {"url": "https://example.com/", "method": "GET"},
            "timestamp": 1.0,
        },
    })
    assert tab.get_network_requests() == []
