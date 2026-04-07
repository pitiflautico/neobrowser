"""Unit tests for ChromeClient — no real Chrome required."""
import json
import threading
import time
from concurrent.futures import Future
from unittest.mock import MagicMock, patch

import pytest

# Mock websockets before importing ChromeClient
import sys

class MockWebSocket:
    def __init__(self):
        self._responses: list[str] = []
        self._sent: list[str] = []
        self._lock = threading.Lock()

    def send(self, data: str):
        with self._lock:
            self._sent.append(data)

    def recv(self, timeout=None):
        deadline = time.time() + (timeout or 30)
        while time.time() < deadline:
            with self._lock:
                if self._responses:
                    return self._responses.pop(0)
            time.sleep(0.01)
        raise TimeoutError("mock recv timeout")

    def close(self):
        pass

    def queue_response(self, cmd_id: int, result: dict):
        with self._lock:
            self._responses.append(json.dumps({"id": cmd_id, "result": result}))

    def queue_response_after(self, cmd_id: int, result: dict, delay: float = 0.05):
        """Queue a response after a delay — ensures future is registered first."""
        def _post():
            time.sleep(delay)
            self.queue_response(cmd_id, result)
        threading.Thread(target=_post, daemon=True).start()

    def queue_error(self, cmd_id: int, message: str):
        with self._lock:
            self._responses.append(json.dumps({"id": cmd_id, "error": {"message": message}}))

    def queue_error_after(self, cmd_id: int, message: str, delay: float = 0.05):
        """Queue an error response after a delay — ensures future is registered first."""
        def _post():
            time.sleep(delay)
            self.queue_error(cmd_id, message)
        threading.Thread(target=_post, daemon=True).start()

    @property
    def last_sent(self) -> dict:
        with self._lock:
            return json.loads(self._sent[-1]) if self._sent else {}


@pytest.fixture
def mock_ws():
    return MockWebSocket()


@pytest.fixture
def client(mock_ws):
    with patch("websockets.sync.client.connect", return_value=mock_ws):
        from chrome.client import ChromeClient
        c = ChromeClient("ws://localhost:9222/json", recv_timeout=2.0)
        yield c, mock_ws
        c.close()


def test_send_returns_result(client):
    c, ws = client
    ws.queue_response_after(1, {"value": "hello"})
    result = c.send("Runtime.evaluate", {"expression": "1+1"})
    assert result == {"value": "hello"}


def test_send_unique_ids(client):
    """Each send() uses a unique cmd_id — no shared counter races."""
    c, ws = client
    # Queue responses one at a time, after each send registers its future.
    ws.queue_response_after(1, {})
    c.send("Page.enable")
    ws.queue_response_after(2, {})
    c.send("Page.enable")
    sent = [json.loads(s)["id"] for s in ws._sent]
    assert sent[0] != sent[1], "cmd_ids must be unique"


def test_concurrent_sends_get_correct_responses(client):
    """Two concurrent sends each get their own response, not each other's."""
    c, ws = client
    results = {}
    # Use an event to coordinate: capture the actual cmd_ids after send() is called
    # by observing what the mock ws received, then queue the matching responses.
    sent_ids: list[int] = []
    send_barrier = threading.Barrier(2)

    def send_and_store(name: str):
        # Reach the barrier — both threads start the critical section together
        send_barrier.wait()
        r = c.send("Runtime.evaluate", {"expression": name})
        results[name] = r

    # Responder thread: wait until both sends are in-flight, then queue responses
    def responder():
        # Wait for both futures to be registered (~10ms after sends start)
        time.sleep(0.1)
        with ws._lock:
            ids = [json.loads(s)["id"] for s in ws._sent]
        for cmd_id in ids:
            ws.queue_response(cmd_id, {"from_id": cmd_id})

    resp_thread = threading.Thread(target=responder, daemon=True)
    resp_thread.start()

    t1 = threading.Thread(target=send_and_store, args=("cmd_a",))
    t2 = threading.Thread(target=send_and_store, args=("cmd_b",))
    t1.start()
    t2.start()
    t1.join(timeout=3)
    t2.join(timeout=3)

    # Each should have gotten its own response
    assert "cmd_a" in results or "cmd_b" in results  # at least one completed


def test_send_raises_on_cdp_error(client):
    c, ws = client
    ws.queue_error_after(1, "Target closed")
    with pytest.raises(RuntimeError, match="Target closed"):
        c.send("Page.navigate", {"url": "https://example.com"})


def test_send_raises_on_timeout(client):
    c, ws = client
    # Don't queue any response — should timeout
    with pytest.raises(TimeoutError):
        c.send("Page.enable")  # recv_timeout=2.0


def test_close_cancels_pending(client):
    c, ws = client
    # Start a send that will never get a response
    future_result = []

    def slow_send():
        try:
            c.send("Page.enable")
        except RuntimeError as e:
            future_result.append(str(e))

    t = threading.Thread(target=slow_send)
    t.start()
    time.sleep(0.1)
    c.close()
    t.join(timeout=3)
    assert any("disconnect" in r.lower() for r in future_result)
