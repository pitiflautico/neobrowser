#!/usr/bin/env python3
"""Compare Chrome CDP vs NeoRender on the same URL.

Usage:
    python3 scripts/compare.py <url>
    python3 scripts/compare.py https://chatgpt.com
    python3 scripts/compare.py https://example.com --timeout 15

Requirements:
    - NeoRender binary at ./target/release/neorender (or set NEORENDER env)
    - Chrome with --remote-debugging-port=9222 (optional, for comparison)
    - pip install websocket-client (optional, for Chrome CDP eval)
"""

import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

# ── Config ──

URL = "https://chatgpt.com"
TIMEOUT = 30
CHROME_PORT = 9222

# Parse args
args = sys.argv[1:]
while args:
    a = args.pop(0)
    if a == "--timeout" and args:
        TIMEOUT = int(args.pop(0))
    elif a == "--port" and args:
        CHROME_PORT = int(args.pop(0))
    elif not a.startswith("-"):
        URL = a

NEORENDER = os.environ.get(
    "NEORENDER",
    str(Path(__file__).resolve().parent.parent / "target" / "release" / "neorender"),
)

# JS snippets for diagnostics
DIAG_JS = """
(function() {
    var d = {};
    d.dom_nodes = document.querySelectorAll('*').length;
    d.react = typeof React;
    d.reactdom = typeof ReactDOM;
    d.next = typeof __NEXT_DATA__;
    d.vite = typeof __vite__mapDeps;
    d.vue = typeof __VUE__;
    d.angular = typeof ng;
    d.svelte = typeof __svelte;
    d.jquery = typeof jQuery;

    // React fibers (sample first 500 elements)
    var els = document.querySelectorAll('*');
    var fibers = 0;
    for (var i = 0; i < Math.min(els.length, 500); i++) {
        var keys = Object.keys(els[i]);
        for (var j = 0; j < keys.length; j++) {
            if (keys[j].startsWith('__react')) { fibers++; break; }
        }
    }
    d.react_fibers = fibers;

    // Globals that look like modules/frameworks
    var interesting = [];
    var skip = new Set(['chrome','ozone','cdc_adoQpoasnfa76pfcZLmcfl_','webdriver']);
    for (var k in window) {
        if (k.startsWith('__') && !skip.has(k)) interesting.push(k);
    }
    d.dunder_globals = interesting.slice(0, 30);
    d.dunder_count = interesting.length;

    // Script tags
    d.scripts_total = document.querySelectorAll('script').length;
    d.scripts_src = document.querySelectorAll('script[src]').length;
    d.scripts_inline = d.scripts_total - d.scripts_src;
    d.scripts_module = document.querySelectorAll('script[type="module"]').length;

    // Errors from window.onerror (won't capture past errors, but useful live)
    d.title = document.title;
    d.url = location.href;
    d.ready_state = document.readyState;

    return JSON.stringify(d);
})()
""".strip()

SEPARATOR = "-" * 60


def section(title):
    print(f"\n{SEPARATOR}")
    print(f"  {title}")
    print(SEPARATOR)


def safe_json(s):
    try:
        return json.loads(s)
    except (json.JSONDecodeError, TypeError):
        return None


# ── NeoRender ──

def run_neorender():
    section("NeoRender")

    if not os.path.isfile(NEORENDER):
        print(f"  SKIP: binary not found at {NEORENDER}")
        print(f"  Build with: cargo build --release")
        return None

    # 1. Navigate via `see`
    print(f"  Loading {URL} ...")
    t0 = time.time()
    try:
        result = subprocess.run(
            [NEORENDER, "see", URL],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT after {TIMEOUT}s")
        return None

    elapsed = time.time() - t0
    data = safe_json(result.stdout)

    if not data:
        print(f"  FAILED ({elapsed:.1f}s)")
        stderr_lines = (result.stderr or "").strip().split("\n")
        for line in stderr_lines[:5]:
            print(f"    {line}")
        return None

    wom = data.get("wom", {})
    errors = data.get("errors", [])
    print(f"  OK ({elapsed:.1f}s, render={data.get('render_ms', '?')}ms)")
    print(f"  Title: {data.get('title', '?')[:70]}")
    print(f"  URL:   {data.get('url', '?')[:70]}")
    print(f"  WOM nodes:  {len(wom.get('nodes', []))}")
    print(f"  Page type:  {wom.get('page_type', '?')}")
    print(f"  Errors:     {len(errors)}")
    for e in errors[:5]:
        print(f"    - {str(e)[:80]}")

    # 2. Run diag JS via interact
    print(f"\n  Running JS diagnostics...")
    try:
        proc = subprocess.run(
            [NEORENDER, "interact", URL],
            input=f"eval {DIAG_JS}\nquit\n",
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
        # Parse eval output — look for JSON in stdout
        for line in proc.stdout.split("\n"):
            stripped = line.strip()
            diag = safe_json(stripped)
            if diag and isinstance(diag, dict) and "dom_nodes" in diag:
                return diag
        # Fallback: try stderr (neo> prompt goes to stderr)
        for line in proc.stderr.split("\n"):
            stripped = line.strip()
            diag = safe_json(stripped)
            if diag and isinstance(diag, dict) and "dom_nodes" in diag:
                return diag

        print(f"  Could not parse diag output")
        # Show raw for debugging
        for line in proc.stdout.split("\n")[:5]:
            if line.strip():
                print(f"    stdout: {line.strip()[:80]}")
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT on interact")
    except Exception as e:
        print(f"  ERROR: {e}")

    return None


# ── Chrome CDP ──

def chrome_available():
    try:
        resp = urllib.request.urlopen(
            f"http://localhost:{CHROME_PORT}/json/version", timeout=2
        )
        return json.loads(resp.read())
    except Exception:
        return None


def start_chrome_hint():
    print(f"  Chrome not available on port {CHROME_PORT}.")
    print(f"  Start with:")
    print(f'    /Applications/Google\\ Chrome.app/Contents/MacOS/Google\\ Chrome \\')
    print(f"      --remote-debugging-port={CHROME_PORT} --headless=new \\")
    print(f"      --disable-gpu --no-first-run \\")
    print(f"      --user-data-dir=/tmp/chrome-debug-profile &")


def run_chrome():
    section("Chrome CDP")

    version = chrome_available()
    if not version:
        start_chrome_hint()
        return None

    browser = version.get("Browser", "?")
    print(f"  Chrome: {browser}")

    # Open new tab
    try:
        resp = urllib.request.urlopen(
            f"http://localhost:{CHROME_PORT}/json/new?{URL}", timeout=5
        )
        tab = json.loads(resp.read())
    except Exception as e:
        print(f"  Failed to open tab: {e}")
        return None

    ws_url = tab.get("webSocketDebuggerUrl")
    tab_id = tab.get("id")
    print(f"  Tab: {tab_id}")
    print(f"  Loading {URL} ...")

    # Wait for page load + hydration
    time.sleep(8)

    diag = None
    try:
        import websocket

        ws = websocket.create_connection(ws_url, timeout=15)

        msg_id = 0

        def cdp_send(method, params=None):
            nonlocal msg_id
            msg_id += 1
            payload = {"id": msg_id, "method": method}
            if params:
                payload["params"] = params
            ws.send(json.dumps(payload))
            # Read responses until we get our id back
            deadline = time.time() + 10
            while time.time() < deadline:
                raw = ws.recv()
                resp = json.loads(raw)
                if resp.get("id") == msg_id:
                    return resp
            return None

        def cdp_eval(expression):
            resp = cdp_send(
                "Runtime.evaluate",
                {"expression": expression, "returnByValue": True},
            )
            if not resp:
                return "TIMEOUT"
            result = resp.get("result", {}).get("result", {})
            if result.get("type") == "string":
                return result.get("value", "")
            return result.get("value", result.get("description", "ERROR"))

        raw = cdp_eval(DIAG_JS)
        diag = safe_json(raw)

        if diag:
            print(f"  OK")
            print(f"  Title: {diag.get('title', '?')[:70]}")
            print(f"  URL:   {diag.get('url', '?')[:70]}")
            print(f"  Ready: {diag.get('ready_state', '?')}")
        else:
            print(f"  Eval returned: {str(raw)[:100]}")

        ws.close()

    except ImportError:
        print(f"  websocket-client not installed.")
        print(f"  Install: pip install websocket-client")
    except Exception as e:
        print(f"  WebSocket error: {e}")

    # Close tab
    try:
        urllib.request.urlopen(
            f"http://localhost:{CHROME_PORT}/json/close/{tab_id}", timeout=2
        )
    except Exception:
        pass

    return diag


# ── Comparison ──

def compare(neo, chrome):
    section("COMPARISON")

    if not neo and not chrome:
        print("  No data from either engine. Nothing to compare.")
        return

    if not neo:
        print("  NeoRender returned no diag data. Only Chrome results available.")
        return

    if not chrome:
        print("  Chrome not available. Only NeoRender results shown.")
        show_single(neo, "NeoRender")
        return

    # Side-by-side
    fields = [
        ("DOM nodes", "dom_nodes"),
        ("React", "react"),
        ("ReactDOM", "reactdom"),
        ("Next.js", "next"),
        ("Vite", "vite"),
        ("Vue", "vue"),
        ("Angular", "angular"),
        ("Svelte", "svelte"),
        ("jQuery", "jquery"),
        ("React fibers", "react_fibers"),
        ("__ globals", "dunder_count"),
        ("Scripts total", "scripts_total"),
        ("Scripts src", "scripts_src"),
        ("Scripts inline", "scripts_inline"),
        ("Scripts module", "scripts_module"),
        ("Ready state", "ready_state"),
    ]

    hdr = f"  {'Metric':<20} {'NeoRender':>15} {'Chrome':>15}  {'Match':>5}"
    print(hdr)
    print(f"  {'─' * 60}")

    matches = 0
    total = 0
    for label, key in fields:
        nv = neo.get(key, "—")
        cv = chrome.get(key, "—")
        eq = "  ==" if str(nv) == str(cv) else "  !="
        if str(nv) == str(cv):
            matches += 1
        total += 1
        print(f"  {label:<20} {str(nv):>15} {str(cv):>15} {eq}")

    print(f"\n  Match rate: {matches}/{total} ({100*matches//total}%)")

    # Show divergent globals
    neo_gl = set(neo.get("dunder_globals", []))
    chrome_gl = set(chrome.get("dunder_globals", []))
    only_chrome = chrome_gl - neo_gl
    only_neo = neo_gl - chrome_gl

    if only_chrome:
        print(f"\n  Globals only in Chrome ({len(only_chrome)}):")
        for g in sorted(only_chrome)[:15]:
            print(f"    + {g}")
    if only_neo:
        print(f"\n  Globals only in NeoRender ({len(only_neo)}):")
        for g in sorted(only_neo)[:15]:
            print(f"    + {g}")


def show_single(data, label):
    print(f"\n  {label} diagnostics:")
    for key in [
        "dom_nodes", "react", "reactdom", "next", "vite", "vue",
        "react_fibers", "dunder_count", "scripts_total", "scripts_src",
        "scripts_module", "ready_state",
    ]:
        val = data.get(key, "—")
        print(f"    {key:<20} {val}")

    globs = data.get("dunder_globals", [])
    if globs:
        print(f"    __ globals: {', '.join(globs[:20])}")


# ── Main ──

def main():
    print(f"=== NeoRender vs Chrome Comparison ===")
    print(f"URL: {URL}")
    print(f"Timeout: {TIMEOUT}s")

    neo_diag = run_neorender()
    chrome_diag = run_chrome()
    compare(neo_diag, chrome_diag)

    print(f"\n{'=' * 60}")
    print(f"Done.")


if __name__ == "__main__":
    main()
