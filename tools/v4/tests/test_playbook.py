"""
tools/v4/tests/test_playbook.py

Unit tests for F10 — ActionRecorder + PlaybookStore + PlaybookRunner.
No Chrome required — PlaybookRunner uses mocked tab + analyzer.
"""
from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from tools.v4.playbook import (
    ActionRecorder,
    PlaybookRunner,
    PlaybookStore,
    Step,
    VALID_ACTIONS,
)


# ---------------------------------------------------------------------------
# Step validation
# ---------------------------------------------------------------------------


def test_step_rejects_invalid_action():
    with pytest.raises(ValueError, match="Invalid action"):
        Step(action="hover", params={})


def test_step_valid_actions():
    for action in VALID_ACTIONS:
        s = Step(action=action, params={})
        assert s.action == action


# ---------------------------------------------------------------------------
# Test 1: ActionRecorder.record() accumulates steps
# ---------------------------------------------------------------------------


def test_recorder_accumulates_steps():
    rec = ActionRecorder()
    s1 = Step("navigate", {"url": "https://example.com"})
    s2 = Step("click_node", {"backend_node_id": 42, "role": "button", "name": "Submit"})
    rec.record(s1)
    rec.record(s2)
    assert len(rec) == 2
    pb = rec.get_playbook()
    assert pb[0] is s1
    assert pb[1] is s2


def test_recorder_get_playbook_returns_copy():
    rec = ActionRecorder()
    rec.record(Step("navigate", {"url": "https://a.com"}))
    pb = rec.get_playbook()
    pb.clear()
    assert len(rec) == 1  # internal state unchanged


def test_recorder_reset_clears_steps():
    rec = ActionRecorder()
    rec.record(Step("navigate", {"url": "https://a.com"}))
    rec.reset()
    assert len(rec) == 0
    assert rec.get_playbook() == []


# ---------------------------------------------------------------------------
# Test 2: PlaybookStore save/load round-trip
# ---------------------------------------------------------------------------


def test_store_save_load_roundtrip(tmp_path):
    store = PlaybookStore(base=tmp_path)
    steps = [
        Step("navigate", {"url": "https://linkedin.com/messaging/"}),
        Step("click_node", {"backend_node_id": 99, "role": "button", "name": "Toni"}, fallback={"role": "button", "name": "Toni"}),
        Step("type", {"text": "Hola!"}),
    ]
    store.save("linkedin.com", "send_message", steps)
    loaded = store.load("linkedin.com", "send_message")
    assert loaded is not None
    assert len(loaded) == 3
    assert loaded[0].action == "navigate"
    assert loaded[1].params["backend_node_id"] == 99
    assert loaded[1].fallback == {"role": "button", "name": "Toni"}
    assert loaded[2].params["text"] == "Hola!"


# ---------------------------------------------------------------------------
# Test 3: PlaybookRunner.run() executes all steps
# ---------------------------------------------------------------------------


def _make_tab() -> MagicMock:
    tab = MagicMock()
    tab.navigate.return_value = None
    tab.wait_for_selector.return_value = True
    tab.js.return_value = None
    # DOM.resolveNode → objectId present
    tab.send.return_value = {"object": {"objectId": "obj-1"}}
    return tab


def _make_analyzer() -> MagicMock:
    return MagicMock()


def test_runner_executes_all_steps():
    tab = _make_tab()
    analyzer = _make_analyzer()
    steps = [
        Step("navigate", {"url": "https://example.com"}),
        Step("wait_selector", {"selector": "body", "timeout_s": 3.0}),
    ]
    runner = PlaybookRunner(wait_after_navigate_s=0)
    ok, first_fail = runner.run(tab, steps, analyzer)
    assert ok is True
    assert first_fail == -1
    tab.navigate.assert_called_once_with("https://example.com", wait_s=0)
    tab.wait_for_selector.assert_called_once_with("body", timeout_s=3.0)


# ---------------------------------------------------------------------------
# Test 4: PlaybookRunner step fails → uses fallback → continues
# ---------------------------------------------------------------------------


def test_runner_fallback_on_click_failure():
    tab = _make_tab()
    # First DOM.resolveNode (stale id) → no objectId → click fails
    # Second DOM.resolveNode (new id) → objectId present
    tab.send.side_effect = [
        {},                                     # resolveNode stale → no object
        {"object": {"objectId": "new-obj"}},    # resolveNode after rediscovery
        {},                                     # callFunctionOn (click)
    ]
    analyzer = _make_analyzer()
    analyzer.find_by_intent.return_value = 200  # new backendNodeId

    steps = [
        Step(
            "click_node",
            {"backend_node_id": 99, "role": "button", "name": "Send"},
            fallback={"role": "button", "name": "Send"},
        )
    ]
    runner = PlaybookRunner()
    ok, first_fail = runner.run(tab, steps, analyzer)

    analyzer.find_by_intent.assert_called_once()
    # step.params updated with new id
    assert steps[0].params["backend_node_id"] == 200


# ---------------------------------------------------------------------------
# Test 5: PlaybookRunner updates backendNodeId after re-discovery
# ---------------------------------------------------------------------------


def test_runner_updates_backend_node_id():
    tab = _make_tab()
    tab.send.side_effect = [
        {},                                   # stale resolveNode → fail
        {"object": {"objectId": "obj-new"}},  # new resolveNode
        {},                                   # callFunctionOn
    ]
    analyzer = _make_analyzer()
    analyzer.find_by_intent.return_value = 777

    step = Step("click_node", {"backend_node_id": 1}, fallback={"role": "textbox", "name": "input"})
    runner = PlaybookRunner()
    runner.run(tab, [step], analyzer)

    assert step.params["backend_node_id"] == 777


# ---------------------------------------------------------------------------
# Test 6: PlaybookStore.load() returns None if file doesn't exist
# ---------------------------------------------------------------------------


def test_store_load_nonexistent_returns_none(tmp_path):
    store = PlaybookStore(base=tmp_path)
    result = store.load("somedomain.com", "no_such_task")
    assert result is None


# ---------------------------------------------------------------------------
# Test 7: domain/task sanitized (no path traversal)
# ---------------------------------------------------------------------------


def test_store_rejects_path_traversal(tmp_path):
    store = PlaybookStore(base=tmp_path)
    with pytest.raises(ValueError):
        store.save("../evil", "task", [])
    with pytest.raises(ValueError):
        store.save("domain", "../etc/passwd", [])
    with pytest.raises(ValueError):
        store.load("/absolute/path", "task")


# ---------------------------------------------------------------------------
# Test 8: permissions 0600 on saved playbook file
# ---------------------------------------------------------------------------


def test_store_save_sets_0600_permissions(tmp_path):
    store = PlaybookStore(base=tmp_path)
    steps = [Step("navigate", {"url": "https://example.com"})]
    store.save("example.com", "test_task", steps)
    path = tmp_path / "example.com" / "test_task.json"
    mode = path.stat().st_mode & 0o777
    assert mode == 0o600


# ---------------------------------------------------------------------------
# Test 9: list_tasks returns correct task names
# ---------------------------------------------------------------------------


def test_store_list_tasks(tmp_path):
    store = PlaybookStore(base=tmp_path)
    steps = [Step("navigate", {"url": "https://a.com"})]
    store.save("domain.com", "task_a", steps)
    store.save("domain.com", "task_b", steps)
    tasks = store.list_tasks("domain.com")
    assert set(tasks) == {"task_a", "task_b"}


def test_store_list_tasks_empty_domain(tmp_path):
    store = PlaybookStore(base=tmp_path)
    assert store.list_tasks("nonexistent.com") == []
