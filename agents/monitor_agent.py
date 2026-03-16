#!/usr/bin/env python3
"""Web app monitoring agent — uses neobrowser-rs via NeoClient.

Usage:
  python3 monitor_agent.py https://app.com          # first run creates baseline
  python3 monitor_agent.py https://app.com           # subsequent runs compare
  python3 monitor_agent.py https://app.com --reset   # reset baseline
"""

import argparse
import asyncio
import json
import os
import sys
import time
from datetime import datetime, timezone
from urllib.parse import urlparse

sys.path.insert(0, "/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/ai-chat")
from aichat.neo_client import NeoClient

BASELINES_DIR = os.path.expanduser("~/.neobrowser/baselines")


def domain_from_url(url: str) -> str:
    return urlparse(url).netloc.replace(":", "_")


def baseline_path(domain: str) -> str:
    return os.path.join(BASELINES_DIR, f"{domain}.json")


def load_baseline(domain: str) -> dict | None:
    p = baseline_path(domain)
    if os.path.exists(p):
        with open(p) as f:
            return json.load(f)
    return None


def save_baseline(domain: str, state: dict):
    os.makedirs(BASELINES_DIR, exist_ok=True)
    with open(baseline_path(domain), "w") as f:
        json.dump(state, f, indent=2)


async def eval_js(neo: NeoClient, code: str) -> str:
    """Run JS via browser_act eval, extract result text."""
    raw = await neo.call_tool("browser_act", {"kind": "eval", "text": code})
    # Format: "eval_result: <value>"
    if raw.startswith("eval_result: "):
        return raw[len("eval_result: "):]
    return raw


async def capture_state(neo: NeoClient, url: str) -> dict:
    """Capture full page state snapshot."""
    t0 = time.monotonic()

    # Open page
    await neo.call_tool("browser_open", {"url": url, "mode": "chrome"})

    # Start network capture
    await neo.call_tool("browser_network", {"op": "start"})
    await neo.call_tool("browser_trace", {"op": "start"})

    # Wait for load to settle
    await asyncio.sleep(2)

    load_time = time.monotonic() - t0

    # Health check
    health_raw = await neo.call_tool("browser_state", {"op": "health"})

    # Page metadata via JS
    title = await eval_js(neo, "document.title")
    body_text = await eval_js(neo, "document.body?.innerText?.substring(0, 500) || ''")
    dom_size = await eval_js(neo, "document.querySelectorAll('*').length")
    cookie_count = await eval_js(neo, "document.cookie.split(';').filter(c => c.trim()).length")

    # Error indicators
    has_login = await eval_js(neo, """
        !!document.querySelector('input[type="password"], form[action*="login"], form[action*="signin"]')
    """)
    has_captcha = await eval_js(neo, """
        !!document.querySelector('[class*="captcha"], [id*="captcha"], iframe[src*="captcha"], iframe[src*="recaptcha"]')
    """)
    console_errors = await eval_js(neo, """
        (window.__monitorErrors || []).length
    """)
    # Inject error listener for next time
    await eval_js(neo, """
        window.__monitorErrors = window.__monitorErrors || [];
        window.addEventListener('error', e => window.__monitorErrors.push(e.message));
    """)

    # HTTP status via performance API
    status_code = await eval_js(neo, """
        (() => {
            const nav = performance.getEntriesByType('navigation')[0];
            return nav ? nav.responseStatus || 200 : 200;
        })()
    """)

    # Network stats
    net_raw = await neo.call_tool("browser_network", {"op": "read"})
    net_requests = _parse_network(net_raw)
    failed_requests = [r for r in net_requests if r.get("failed")]
    slow_requests = [r for r in net_requests if r.get("duration_ms", 0) > 5000]

    # Trace stats
    trace_raw = await neo.call_tool("browser_trace", {"op": "stats"})

    return {
        "url": url,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "title": title,
        "body_text": body_text[:500],
        "dom_size": _safe_int(dom_size),
        "cookie_count": _safe_int(cookie_count),
        "status_code": _safe_int(status_code, 200),
        "load_time_s": round(load_time, 2),
        "health": health_raw,
        "has_login_form": has_login.strip().lower() == "true",
        "has_captcha": has_captcha.strip().lower() == "true",
        "console_errors": _safe_int(console_errors),
        "network_request_count": len(net_requests),
        "failed_requests": [r.get("url", "?") for r in failed_requests],
        "slow_requests": [r.get("url", "?") for r in slow_requests],
        "trace": trace_raw,
    }


def _safe_int(val, default=0) -> int:
    try:
        return int(val)
    except (ValueError, TypeError):
        return default


def _parse_network(raw: str) -> list[dict]:
    """Best-effort parse of network capture output."""
    try:
        data = json.loads(raw)
        if isinstance(data, list):
            return data
        if isinstance(data, dict) and "requests" in data:
            return data["requests"]
    except (json.JSONDecodeError, TypeError):
        pass
    # Count lines as rough request count
    lines = [l for l in raw.strip().splitlines() if l.strip() and not l.startswith("#")]
    return [{"url": l.strip()} for l in lines]


def diff_states(current: dict, baseline: dict) -> dict:
    """Compare current state against baseline, return changes."""
    changes = []

    if current["title"] != baseline.get("title"):
        changes.append(f"Title changed: '{baseline.get('title')}' -> '{current['title']}'")

    bl_body = baseline.get("body_text", "")
    if current["body_text"][:200] != bl_body[:200]:
        changes.append("Key content changed (first 200 chars differ)")

    bl_cookies = baseline.get("cookie_count", 0)
    cur_cookies = current["cookie_count"]
    if bl_cookies > 0 and abs(cur_cookies - bl_cookies) / max(bl_cookies, 1) > 0.5:
        changes.append(f"Cookie count changed significantly: {bl_cookies} -> {cur_cookies}")

    if current["console_errors"] > baseline.get("console_errors", 0):
        changes.append(f"New console errors: {baseline.get('console_errors', 0)} -> {current['console_errors']}")

    bl_load = baseline.get("load_time_s", 0)
    if bl_load > 0 and current["load_time_s"] > bl_load * 2:
        changes.append(f"Performance degraded: {bl_load}s -> {current['load_time_s']}s (>{2}x)")

    if current["failed_requests"] and not baseline.get("failed_requests"):
        changes.append(f"New failed requests: {len(current['failed_requests'])}")

    return {"changes": changes, "change_count": len(changes)}


def determine_status(state: dict, diff: dict | None) -> str:
    """Classify site health: healthy / degraded / down."""
    if not state["title"] or state["status_code"] >= 500:
        return "down"
    if state["has_captcha"]:
        return "degraded"
    if state["has_login_form"]:
        return "degraded"
    if state["console_errors"] > 10:
        return "degraded"
    if len(state["failed_requests"]) > 5:
        return "degraded"
    if diff and diff["change_count"] > 3:
        return "degraded"
    return "healthy"


def build_report(state: dict, diff: dict | None, status: str, is_baseline: bool) -> dict:
    return {
        "status": status,
        "url": state["url"],
        "timestamp": state["timestamp"],
        "is_baseline_run": is_baseline,
        "performance": {
            "load_time_s": state["load_time_s"],
            "dom_size": state["dom_size"],
            "network_requests": state["network_request_count"],
        },
        "errors": {
            "console_errors": state["console_errors"],
            "failed_requests": state["failed_requests"],
            "slow_requests": state["slow_requests"],
        },
        "indicators": {
            "has_login_form": state["has_login_form"],
            "has_captcha": state["has_captcha"],
        },
        "changes": diff["changes"] if diff else [],
        "change_count": diff["change_count"] if diff else 0,
    }


def save_report(report: dict, domain: str):
    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    path = f"/tmp/monitor_{domain}_{ts}.json"
    with open(path, "w") as f:
        json.dump(report, f, indent=2)
    return path


async def run(url: str, reset: bool = False):
    domain = domain_from_url(url)
    neo = NeoClient()

    try:
        await neo.start()
        print(f"[monitor] Capturing state for {url} ...")

        state = await capture_state(neo, url)
        baseline = None if reset else load_baseline(domain)
        diff = None

        if baseline:
            diff = diff_states(state, baseline)
            print(f"[monitor] Compared against baseline from {baseline.get('timestamp', '?')}")
            is_baseline = False
        else:
            save_baseline(domain, state)
            print(f"[monitor] Baseline saved to {baseline_path(domain)}")
            is_baseline = True

        status = determine_status(state, diff)
        report = build_report(state, diff, status, is_baseline)
        report_path = save_report(report, domain)

        # Summary
        icon = {"healthy": "OK", "degraded": "WARN", "down": "FAIL"}[status]
        print(f"\n[{icon}] {status.upper()} — {url}")
        print(f"  Load: {state['load_time_s']}s | DOM: {state['dom_size']} | Net: {state['network_request_count']} reqs")
        if state["failed_requests"]:
            print(f"  Failed requests: {len(state['failed_requests'])}")
        if state["console_errors"]:
            print(f"  Console errors: {state['console_errors']}")
        if diff and diff["changes"]:
            print(f"  Changes ({diff['change_count']}):")
            for c in diff["changes"]:
                print(f"    - {c}")
        print(f"\n  Report: {report_path}")

    finally:
        await neo.stop()


def main():
    parser = argparse.ArgumentParser(description="Web app monitoring agent")
    parser.add_argument("url", help="Target URL to monitor")
    parser.add_argument("--reset", action="store_true", help="Reset baseline")
    args = parser.parse_args()
    asyncio.run(run(args.url, args.reset))


if __name__ == "__main__":
    main()
