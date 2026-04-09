"""
tools/v4/playbook.py

F10 — ActionRecorder + Playbook Engine

Cognitive learning layer: record page interaction steps on first visit,
replay them on subsequent visits — zero AX discovery cost on replay.

If a step fails during replay, PlaybookRunner falls back to PageAnalyzer
re-discovery, updates the stale backendNodeId, and continues.

Storage: ~/.neorender/playbooks/{domain}/{task}.json  perms 0600
"""
from __future__ import annotations

import json
import logging
import re
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tools.v4.chrome_tab import ChromeTab
    from tools.v4.page_analyzer import PageAnalyzer

log = logging.getLogger(__name__)

PLAYBOOKS_BASE = Path.home() / ".neorender" / "playbooks"

# Only allow simple alphanumeric names with hyphens/underscores (path traversal guard).
_SAFE_NAME_RE = re.compile(r'^[a-zA-Z0-9][a-zA-Z0-9_\-\.]{0,127}$')

VALID_ACTIONS = frozenset({"navigate", "click_node", "type", "wait_selector"})


def _validate_name(name: str, label: str) -> None:
    """Raise ValueError if name could cause path traversal."""
    if not _SAFE_NAME_RE.match(name):
        raise ValueError(
            f"Invalid {label} {name!r}. "
            "Only letters, digits, hyphens, underscores, and dots allowed (max 128 chars)."
        )


# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


@dataclass
class Step:
    """
    A single recorded interaction step.

    action: "navigate" | "click_node" | "type" | "wait_selector"
    params: action-specific payload
      - navigate:      {"url": str}
      - click_node:    {"backend_node_id": int, "role": str, "name": str}
      - type:          {"text": str}
      - wait_selector: {"selector": str, "timeout_s": float}
    fallback: for click_node — {"role": str, "name": str} to re-discover
              via PageAnalyzer if backend_node_id is stale. None otherwise.
    """
    action: str
    params: dict = field(default_factory=dict)
    fallback: dict | None = None

    def __post_init__(self) -> None:
        if self.action not in VALID_ACTIONS:
            raise ValueError(
                f"Invalid action {self.action!r}. Must be one of: {sorted(VALID_ACTIONS)}"
            )


# ---------------------------------------------------------------------------
# ActionRecorder
# ---------------------------------------------------------------------------


class ActionRecorder:
    """
    Records Steps in order during a live session.
    Call record() after each successful action.
    Call get_playbook() to retrieve the accumulated steps.
    Call reset() to start a new recording.
    """

    def __init__(self) -> None:
        self._steps: list[Step] = []

    def record(self, step: Step) -> None:
        """Append a step to the recording."""
        self._steps.append(step)
        log.debug("ActionRecorder.record: %s %s", step.action, step.params)

    def get_playbook(self) -> list[Step]:
        """Return a copy of recorded steps."""
        return list(self._steps)

    def reset(self) -> None:
        """Clear all recorded steps."""
        self._steps.clear()

    def __len__(self) -> int:
        return len(self._steps)


# ---------------------------------------------------------------------------
# PlaybookStore
# ---------------------------------------------------------------------------


class PlaybookStore:
    """
    Persist and load playbooks as JSON under ~/.neorender/playbooks/.

    Path structure: {base}/{domain}/{task}.json
    File permissions: 0600 (owner read/write only).
    domain and task are sanitized to prevent path traversal.
    """

    def __init__(self, base: Path | None = None) -> None:
        self._base = base or PLAYBOOKS_BASE

    def _path(self, domain: str, task: str) -> Path:
        _validate_name(domain, "domain")
        _validate_name(task, "task")
        path = (self._base / domain / f"{task}.json").resolve()
        # Extra guard: ensure resolved path is under base
        if not path.is_relative_to(self._base.resolve()):
            raise ValueError(
                f"Path traversal detected: resolved {path} is outside {self._base}"
            )
        return path

    def save(self, domain: str, task: str, steps: list[Step]) -> None:
        """
        Serialize steps to JSON and write to disk with 0600 permissions.
        Creates parent directory if needed.
        """
        path = self._path(domain, task)
        path.parent.mkdir(parents=True, exist_ok=True)
        data = [asdict(s) for s in steps]
        path.write_text(json.dumps(data, indent=2), encoding="utf-8")
        path.chmod(0o600)
        log.debug("PlaybookStore.save: %d steps → %s", len(steps), path)

    def load(self, domain: str, task: str) -> list[Step] | None:
        """
        Load steps from disk. Returns None if file does not exist.
        Returns None (logs warning) if file is corrupt.
        """
        path = self._path(domain, task)
        if not path.exists():
            return None
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            steps = [Step(**item) for item in data]
            log.debug("PlaybookStore.load: %d steps ← %s", len(steps), path)
            return steps
        except Exception as exc:
            log.warning("PlaybookStore.load failed for %s/%s: %s", domain, task, exc)
            return None

    def list_tasks(self, domain: str) -> list[str]:
        """
        Return list of task names saved for domain.
        Returns [] if domain directory doesn't exist.
        """
        _validate_name(domain, "domain")
        domain_dir = self._base / domain
        if not domain_dir.exists():
            return []
        return [f.stem for f in domain_dir.glob("*.json") if f.is_file()]

    def delete(self, domain: str, task: str) -> bool:
        """Delete a playbook. Returns True if deleted, False if not found."""
        path = self._path(domain, task)
        if path.exists():
            path.unlink()
            return True
        return False


# ---------------------------------------------------------------------------
# PlaybookRunner
# ---------------------------------------------------------------------------


class PlaybookRunner:
    """
    Execute a list of Steps against a live ChromeTab.

    On step failure:
      1. If step has a fallback dict (role + name), ask PageAnalyzer to
         re-discover the element and update step.params["backend_node_id"].
      2. Retry the step once with the new node id.
      3. If still fails, mark as failed and continue (best-effort).

    Returns (all_ok: bool, first_failed_index: int)  — first_failed_index=-1 if all ok.
    """

    def __init__(self, wait_after_navigate_s: float = 2.0) -> None:
        self.wait_after_navigate_s = wait_after_navigate_s

    def run(
        self,
        tab: "ChromeTab",
        steps: list[Step],
        analyzer: "PageAnalyzer",
    ) -> tuple[bool, int]:
        """
        Execute all steps in order.

        Returns (all_ok, first_failed_index) where first_failed_index is -1
        if all steps succeeded.
        """
        first_failed = -1
        for i, step in enumerate(steps):
            ok = self._execute_step(tab, step, analyzer)
            if not ok:
                log.warning("PlaybookRunner: step %d failed (%s %s)", i, step.action, step.params)
                if first_failed == -1:
                    first_failed = i
        return (first_failed == -1), first_failed

    def _execute_step(
        self,
        tab: "ChromeTab",
        step: Step,
        analyzer: "PageAnalyzer",
    ) -> bool:
        """Execute a single step. Returns True on success."""
        try:
            if step.action == "navigate":
                tab.navigate(step.params["url"], wait_s=self.wait_after_navigate_s)
                return True

            elif step.action == "click_node":
                node_id = step.params.get("backend_node_id")
                ok = self._click_node(tab, node_id)
                if ok:
                    return True
                # Fallback: re-discover via PageAnalyzer
                if step.fallback:
                    new_id = self._rediscover(tab, step, analyzer)
                    if new_id is not None:
                        step.params["backend_node_id"] = new_id  # update in-place
                        return self._click_node(tab, new_id)
                return False

            elif step.action == "type":
                text = step.params.get("text", "")
                # Use CDP Input.insertText — fires React synthetic events correctly.
                # .value= direct assignment does not trigger React onChange.
                tab.send("Input.insertText", {"text": text})
                return True

            elif step.action == "wait_selector":
                selector = step.params.get("selector", "")
                timeout_s = step.params.get("timeout_s", 5.0)
                return tab.wait_for_selector(selector, timeout_s=timeout_s)

        except Exception as exc:
            log.warning("PlaybookRunner._execute_step exception: %s", exc)
            return False

        return False

    def _click_node(self, tab: "ChromeTab", backend_node_id: int | None) -> bool:
        """Click a DOM node by backendNodeId via CDP DOM.focus + dispatchMouseEvent."""
        if backend_node_id is None:
            return False
        try:
            # Resolve backendNodeId to objectId, then call click()
            result = tab.send("DOM.resolveNode", {"backendNodeId": backend_node_id})
            obj = result.get("object", {})
            obj_id = obj.get("objectId")
            if not obj_id:
                return False
            tab.send("Runtime.callFunctionOn", {
                "objectId": obj_id,
                "functionDeclaration": "function(){this.click()}",
                "returnByValue": True,
            })
            return True
        except Exception as exc:
            log.debug("_click_node failed for id=%s: %s", backend_node_id, exc)
            return False

    def _rediscover(
        self,
        tab: "ChromeTab",
        step: Step,
        analyzer: "PageAnalyzer",
    ) -> int | None:
        """
        Use PageAnalyzer to find an element matching step.fallback {role, name}.
        Returns new backendNodeId or None.
        """
        fallback = step.fallback
        if not fallback:
            return None
        intent = f"{fallback.get('role', '')} {fallback.get('name', '')}".strip()
        if not intent:
            return None
        log.debug("PlaybookRunner._rediscover: intent=%r", intent)
        return analyzer.find_by_intent(tab, intent)
