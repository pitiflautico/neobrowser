"""
PoC 5 — Playbook record → replay

Prueba que:
1. record_task + record_step + stop_recording guarda JSON en disco
2. El archivo JSON tiene los pasos correctos y perms 0600
3. replay() ejecuta todos los pasos contra tab mock
4. replay() retorna (True, -1) cuando todos los pasos van bien
5. Fallback: step con backend_node_id stale → PageAnalyzer re-discovers → continúa
6. Browser.record_step() lanza RuntimeError sin record_task() previo
"""
from __future__ import annotations

import json
import stat
from pathlib import Path
from unittest.mock import MagicMock, call, patch

import pytest

from tools.v4.chrome_tab import ChromeTab
from tools.v4.playbook import (
    ActionRecorder,
    PlaybookRunner,
    PlaybookStore,
    Step,
)


# ─── Helpers ──────────────────────────────────────────────────────────────────

def _make_tab() -> ChromeTab:
    ws = MagicMock()
    tab = ChromeTab(ws=ws, tab_id="t1", port=9222)
    tab.navigate = MagicMock()
    tab.send = MagicMock(return_value={})
    tab.wait_for_selector = MagicMock(return_value=True)
    return tab


def _make_analyzer(node_id: int | None = 42) -> MagicMock:
    a = MagicMock()
    a.find_by_intent.return_value = node_id
    return a


# ─── Unit tests ───────────────────────────────────────────────────────────────

class TestActionRecorder:

    def test_record_appends_steps(self):
        rec = ActionRecorder()
        rec.record(Step("navigate", {"url": "https://example.com"}))
        rec.record(Step("type", {"text": "hello"}))
        assert len(rec) == 2

    def test_get_playbook_returns_copy(self):
        rec = ActionRecorder()
        s = Step("navigate", {"url": "https://x.com"})
        rec.record(s)
        pb = rec.get_playbook()
        pb.clear()
        assert len(rec) == 1  # original not affected

    def test_reset_clears_steps(self):
        rec = ActionRecorder()
        rec.record(Step("type", {"text": "x"}))
        rec.reset()
        assert len(rec) == 0

    def test_invalid_action_raises(self):
        with pytest.raises(ValueError, match="Invalid action"):
            Step("fly_to_moon", {})


class TestPlaybookStore:

    def test_save_and_load_roundtrip(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        steps = [
            Step("navigate", {"url": "https://linkedin.com/messaging/"}),
            Step("type", {"text": "hola"}),
            Step("click_node", {"backend_node_id": 42, "role": "button", "name": "Enviar"},
                 fallback={"role": "button", "name": "Enviar"}),
        ]
        store.save("linkedin.com", "reply_dm", steps)
        loaded = store.load("linkedin.com", "reply_dm")

        assert loaded is not None
        assert len(loaded) == 3
        assert loaded[0].action == "navigate"
        assert loaded[2].fallback == {"role": "button", "name": "Enviar"}

    def test_save_creates_0600_file(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        store.save("test.com", "task1", [Step("navigate", {"url": "https://test.com"})])
        path = tmp_path / "test.com" / "task1.json"
        mode = oct(stat.S_IMODE(path.stat().st_mode))
        assert mode == "0o600"

    def test_load_returns_none_for_missing(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        assert store.load("ghost.com", "nope") is None

    def test_load_returns_none_for_corrupt_json(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        path = tmp_path / "bad.com" / "broken.json"
        path.parent.mkdir(parents=True)
        path.write_text("}{invalid")
        assert store.load("bad.com", "broken") is None

    def test_path_traversal_rejected(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        with pytest.raises(ValueError):
            store.save("../etc", "passwd", [])

    def test_list_tasks(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        store.save("linkedin.com", "task_a", [Step("navigate", {"url": "https://x.com"})])
        store.save("linkedin.com", "task_b", [Step("type", {"text": "x"})])
        tasks = store.list_tasks("linkedin.com")
        assert set(tasks) == {"task_a", "task_b"}

    def test_delete_removes_file(self, tmp_path):
        store = PlaybookStore(base=tmp_path)
        store.save("x.com", "del_me", [Step("navigate", {"url": "https://x.com"})])
        assert store.delete("x.com", "del_me") is True
        assert store.load("x.com", "del_me") is None


class TestPlaybookRunner:

    def test_navigate_step_calls_tab_navigate(self):
        tab = _make_tab()
        analyzer = _make_analyzer()
        runner = PlaybookRunner()
        steps = [Step("navigate", {"url": "https://linkedin.com/messaging/"})]

        ok, first_fail = runner.run(tab, steps, analyzer)

        assert ok is True
        assert first_fail == -1
        tab.navigate.assert_called_once_with(
            "https://linkedin.com/messaging/", wait_s=runner.wait_after_navigate_s
        )

    def test_type_step_uses_input_inserttext(self):
        tab = _make_tab()
        runner = PlaybookRunner()
        steps = [Step("type", {"text": "hello world"})]

        ok, _ = runner.run(tab, steps, _make_analyzer())

        assert ok is True
        tab.send.assert_called_once_with("Input.insertText", {"text": "hello world"})

    def test_click_node_step_resolves_and_clicks(self):
        tab = _make_tab()
        tab.send = MagicMock(side_effect=[
            {"object": {"objectId": "OBJ1"}},  # DOM.resolveNode
            {},                                 # Runtime.callFunctionOn
        ])
        runner = PlaybookRunner()
        steps = [Step("click_node", {"backend_node_id": 99})]

        ok, _ = runner.run(tab, steps, _make_analyzer())

        assert ok is True
        tab.send.assert_any_call("DOM.resolveNode", {"backendNodeId": 99})

    def test_stale_node_id_triggers_fallback_rediscovery(self):
        """click_node with bad nodeId → PageAnalyzer re-discovers → success."""
        tab = _make_tab()
        # First DOM.resolveNode returns no objectId (stale node)
        # After re-discovery, second call returns valid objectId
        tab.send = MagicMock(side_effect=[
            {"object": {}},                     # DOM.resolveNode stale
            {"object": {"objectId": "OBJ2"}},  # DOM.resolveNode after rediscovery
            {},                                 # Runtime.callFunctionOn
        ])
        analyzer = _make_analyzer(node_id=77)  # PageAnalyzer finds node 77
        runner = PlaybookRunner()
        steps = [Step(
            "click_node",
            {"backend_node_id": 1},  # stale
            fallback={"role": "button", "name": "Enviar"},
        )]

        ok, _ = runner.run(tab, steps, analyzer)

        assert ok is True
        analyzer.find_by_intent.assert_called_once()

    def test_all_steps_fail_returns_first_failed(self):
        tab = _make_tab()
        tab.navigate = MagicMock(side_effect=Exception("nav failed"))
        tab.send = MagicMock(return_value={"object": {}})
        runner = PlaybookRunner()
        steps = [
            Step("navigate", {"url": "https://x.com"}),
            Step("type", {"text": "hi"}),
        ]

        ok, first_fail = runner.run(tab, steps, _make_analyzer())

        assert ok is False
        assert first_fail == 0  # first step failed


class TestBrowserPlaybookFacade:

    def test_record_step_requires_active_recording(self):
        from tools.v4.browser import Browser
        with Browser.connect(9999) as b:
            with pytest.raises(RuntimeError, match="record_task"):
                b.record_step(Step("type", {"text": "x"}))

    def test_full_record_stop_cycle(self, tmp_path):
        from tools.v4.browser import Browser
        with Browser.connect(9999) as b:
            b._store = PlaybookStore(base=tmp_path)
            b.record_task("linkedin.com", "test_flow")
            b.record_step(Step("navigate", {"url": "https://linkedin.com/messaging/"}))
            b.record_step(Step("type", {"text": "hola"}))
            steps = b.stop_recording()

        assert len(steps) == 2
        loaded = PlaybookStore(base=tmp_path).load("linkedin.com", "test_flow")
        assert loaded is not None
        assert len(loaded) == 2

    def test_replay_returns_false_for_missing_playbook(self, tmp_path):
        from tools.v4.browser import Browser
        tab = _make_tab()
        with Browser.connect(9999) as b:
            b._store = PlaybookStore(base=tmp_path)
            ok, first_fail = b.replay(tab, "unknown.com", "nonexistent")
        assert ok is False
        assert first_fail == -1

    def test_replay_executes_saved_playbook(self, tmp_path):
        from tools.v4.browser import Browser
        tab = _make_tab()
        store = PlaybookStore(base=tmp_path)
        store.save("test.com", "flow", [
            Step("navigate", {"url": "https://test.com"}),
            Step("type", {"text": "hello"}),
        ])

        with Browser.connect(9999) as b:
            b._store = store
            ok, first_fail = b.replay(tab, "test.com", "flow")

        assert ok is True
        assert first_fail == -1
        tab.navigate.assert_called_once_with("https://test.com", wait_s=b._runner.wait_after_navigate_s)
        tab.send.assert_called_once_with("Input.insertText", {"text": "hello"})
