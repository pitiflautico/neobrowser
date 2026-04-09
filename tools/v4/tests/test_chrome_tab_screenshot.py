"""
tools/v4/tests/test_chrome_tab_screenshot.py

Unit tests for F04 — Screenshot on ChromeTab.
All tests use a mock WebSocket — no Chrome required.
"""
from __future__ import annotations

import base64
import json
import queue
from pathlib import Path

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Mock WebSocket
# ---------------------------------------------------------------------------


class MockWS:
    def __init__(self, screenshot_b64: str = ""):
        self._screenshot_b64 = screenshot_b64
        self.sent: list[dict] = []

    def send(self, payload: str) -> None:
        msg = json.loads(payload)
        self.sent.append(msg)
        # For captureScreenshot, respond with data
        if msg.get("method") == "Page.captureScreenshot":
            response = json.dumps({"id": msg["id"], "result": {"data": self._screenshot_b64}})
        else:
            response = json.dumps({"id": msg["id"], "result": {}})
        self._recv_buf = response

    def recv(self, timeout: float = 30.0) -> str:
        return self._recv_buf

    def close(self) -> None:
        pass


_PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
_FAKE_PNG = _PNG_MAGIC + b"\x00" * 100
_FAKE_PNG_B64 = base64.b64encode(_FAKE_PNG).decode("ascii")

_FAKE_JPEG = b"\xff\xd8\xff" + b"\x00" * 80
_FAKE_JPEG_B64 = base64.b64encode(_FAKE_JPEG).decode("ascii")


def make_tab(b64: str = _FAKE_PNG_B64) -> ChromeTab:
    ws = MockWS(screenshot_b64=b64)
    return ChromeTab(ws=ws, tab_id="test-tab", port=9222)


# ---------------------------------------------------------------------------
# Test 1: screenshot() returns correct bytes
# ---------------------------------------------------------------------------


def test_screenshot_returns_bytes():
    tab = make_tab()
    data = tab.screenshot()
    assert isinstance(data, bytes)
    assert data == _FAKE_PNG


# ---------------------------------------------------------------------------
# Test 2: screenshot_base64() returns valid base64 decodable to same bytes
# ---------------------------------------------------------------------------


def test_screenshot_base64_round_trips():
    tab = make_tab()
    b64 = tab.screenshot_base64()
    assert isinstance(b64, str)
    decoded = base64.b64decode(b64)
    assert decoded == _FAKE_PNG


# ---------------------------------------------------------------------------
# Test 3: format="jpeg" sends quality param to CDP
# ---------------------------------------------------------------------------


def test_screenshot_jpeg_sends_quality():
    tab = make_tab(_FAKE_JPEG_B64)
    tab.screenshot(format="jpeg", quality=70)
    cap_calls = [m for m in tab._ws.sent if m["method"] == "Page.captureScreenshot"]
    assert len(cap_calls) == 1
    assert cap_calls[0]["params"]["format"] == "jpeg"
    assert cap_calls[0]["params"]["quality"] == 70


# ---------------------------------------------------------------------------
# Test 4: format="png" does NOT send quality param
# ---------------------------------------------------------------------------


def test_screenshot_png_no_quality_param():
    tab = make_tab()
    tab.screenshot(format="png")
    cap_calls = [m for m in tab._ws.sent if m["method"] == "Page.captureScreenshot"]
    assert len(cap_calls) == 1
    assert "quality" not in cap_calls[0]["params"]


# ---------------------------------------------------------------------------
# Test 5: unsupported format raises ValueError
# ---------------------------------------------------------------------------


def test_screenshot_unsupported_format_raises():
    tab = make_tab()
    with pytest.raises(ValueError, match="Unsupported screenshot format"):
        tab.screenshot(format="webp")


# ---------------------------------------------------------------------------
# Test 6: screenshot_save() writes bytes to path, returns Path
# ---------------------------------------------------------------------------


def test_screenshot_save_writes_file(tmp_path):
    tab = make_tab()
    dest = tmp_path / "out.png"
    result = tab.screenshot_save(str(dest))
    assert result == dest.resolve()
    assert dest.exists()
    assert dest.read_bytes() == _FAKE_PNG


# ---------------------------------------------------------------------------
# Test 7: screenshot_save() creates parent directories
# ---------------------------------------------------------------------------


def test_screenshot_save_creates_parents(tmp_path):
    tab = make_tab()
    dest = tmp_path / "deep" / "nested" / "shot.png"
    tab.screenshot_save(dest)
    assert dest.exists()
    assert dest.read_bytes() == _FAKE_PNG


# ---------------------------------------------------------------------------
# Test 8: screenshot() result is non-empty bytes
# ---------------------------------------------------------------------------


def test_screenshot_non_empty():
    tab = make_tab()
    data = tab.screenshot()
    assert len(data) > 0
