"""
tools/v4/tests/test_chrome_tab_performance.py

Unit tests for F05 — Performance Metrics on ChromeTab.
All tests use a mock WebSocket — no Chrome required.
"""
from __future__ import annotations

import json
import queue

import pytest

from tools.v4.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Mock WebSocket
# ---------------------------------------------------------------------------

_FAKE_METRICS = [
    {"name": "JSHeapUsedSize",      "value": 5_000_000.0},
    {"name": "JSHeapTotalSize",     "value": 10_000_000.0},
    {"name": "Nodes",               "value": 123.0},
    {"name": "Documents",           "value": 2.0},
    {"name": "Frames",              "value": 1.0},
    {"name": "TaskDuration",        "value": 0.42},
    {"name": "LayoutCount",         "value": 7.0},
    {"name": "RecalcStyleCount",    "value": 4.0},
]


class MockWS:
    def __init__(self):
        self.sent: list[dict] = []

    def send(self, payload: str) -> None:
        msg = json.loads(payload)
        self.sent.append(msg)
        if msg.get("method") == "Performance.getMetrics":
            self._last = json.dumps({"id": msg["id"], "result": {"metrics": _FAKE_METRICS}})
        else:
            self._last = json.dumps({"id": msg["id"], "result": {}})

    def recv(self, timeout: float = 30.0) -> str:
        return self._last

    def close(self) -> None:
        pass


def make_tab() -> ChromeTab:
    return ChromeTab(ws=MockWS(), tab_id="perf-tab", port=9222)


# ---------------------------------------------------------------------------
# Test 1: enable_performance() sends Performance.enable
# ---------------------------------------------------------------------------


def test_enable_performance_sends_cdp():
    tab = make_tab()
    tab.enable_performance()
    calls = [m for m in tab._ws.sent if m["method"] == "Performance.enable"]
    assert len(calls) == 1


# ---------------------------------------------------------------------------
# Test 2: enable_performance() is idempotent
# ---------------------------------------------------------------------------


def test_enable_performance_idempotent():
    tab = make_tab()
    tab.enable_performance()
    tab.enable_performance()
    calls = [m for m in tab._ws.sent if m["method"] == "Performance.enable"]
    assert len(calls) == 1


# ---------------------------------------------------------------------------
# Test 3: get_metrics() returns dict with string keys and float values
# ---------------------------------------------------------------------------


def test_get_metrics_returns_typed_dict():
    tab = make_tab()
    tab.enable_performance()
    metrics = tab.get_metrics()
    assert isinstance(metrics, dict)
    for k, v in metrics.items():
        assert isinstance(k, str)
        assert isinstance(v, float)


# ---------------------------------------------------------------------------
# Test 4: get_metrics() before enable returns {}
# ---------------------------------------------------------------------------


def test_get_metrics_before_enable_returns_empty():
    tab = make_tab()
    assert tab.get_metrics() == {}


# ---------------------------------------------------------------------------
# Test 5: get_metrics() calls Performance.getMetrics via CDP
# ---------------------------------------------------------------------------


def test_get_metrics_calls_cdp():
    tab = make_tab()
    tab.enable_performance()
    tab.get_metrics()
    calls = [m for m in tab._ws.sent if m["method"] == "Performance.getMetrics"]
    assert len(calls) == 1


# ---------------------------------------------------------------------------
# Test 6: get_metric("JSHeapUsedSize") returns correct float
# ---------------------------------------------------------------------------


def test_get_metric_known_key():
    tab = make_tab()
    tab.enable_performance()
    val = tab.get_metric("JSHeapUsedSize")
    assert val == 5_000_000.0


# ---------------------------------------------------------------------------
# Test 7: get_metric("NonExistentMetric") returns None
# ---------------------------------------------------------------------------


def test_get_metric_unknown_key_returns_none():
    tab = make_tab()
    tab.enable_performance()
    assert tab.get_metric("NonExistentMetric") is None


# ---------------------------------------------------------------------------
# Test 8: get_metrics() flattens CDP list correctly
# ---------------------------------------------------------------------------


def test_get_metrics_flattens_list():
    tab = make_tab()
    tab.enable_performance()
    metrics = tab.get_metrics()
    assert metrics["Nodes"] == 123.0
    assert metrics["Frames"] == 1.0
    assert metrics["TaskDuration"] == pytest.approx(0.42)
    assert len(metrics) == len(_FAKE_METRICS)
