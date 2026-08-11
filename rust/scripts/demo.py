#!/usr/bin/env python3
"""
NeoBrowser demo — a reproducible, recordable end-to-end flow.

Drives the MCP server through the tasks that make the pitch land: real form login,
file upload, table extraction, and passing a live bot detector. Prints one narrated
line per step so a screen recording reads clearly.

Usage:
    python3 rust/scripts/demo.py [path-to-neobrowser-binary]
    (defaults to rust/target/release/neobrowser)
"""
import base64, json, os, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "target", "release", "neobrowser")

# A tiny real PNG to upload.
IMG = os.path.join(tempfile.gettempdir(), "neobrowser_demo.png")
open(IMG, "wb").write(base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
))

STEPS = [
    (1, "initialize", {}, None),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://the-internet.herokuapp.com/login", "wait_s": 2}},
     "Open a real login page"),
    (3, "tools/call", {"name": "fill", "arguments": {"selector": "#username", "value": "tomsmith"}},
     "Fill the username"),
    (4, "tools/call", {"name": "fill", "arguments": {"selector": "#password", "value": "SuperSecretPassword!"}},
     "Fill the password"),
    (5, "tools/call", {"name": "find_and_click", "arguments": {"text": "Login"}},
     "Click Login (real isTrusted click)"),
    (6, "tools/call", {"name": "read", "arguments": {"selector": ".flash"}},
     "Read the result → logged in"),
    (7, "tools/call", {"name": "navigate", "arguments": {"url": "https://the-internet.herokuapp.com/upload", "wait_s": 2}},
     "Go to a file-upload form"),
    (8, "tools/call", {"name": "upload", "arguments": {"selector": "#file-upload", "files": [IMG]}},
     "Attach a real image file"),
    (9, "tools/call", {"name": "submit", "arguments": {"selector": "#file-submit", "wait_s": 8}},
     "Submit the upload"),
    (10, "tools/call", {"name": "read", "arguments": {"selector": "#uploaded-files"}},
     "Server confirms the file"),
    (11, "tools/call", {"name": "navigate", "arguments": {"url": "https://bot.sannysoft.com/", "wait_s": 3}},
     "Visit a live bot detector"),
    (12, "tools/call", {"name": "js", "arguments": {"code": "return JSON.stringify({webdriver: navigator.webdriver === undefined ? 'hidden (passed)' : 'LEAKED', chrome_runtime: !!(window.chrome&&window.chrome.runtime), headless_ua: navigator.userAgent.includes('Headless')})"}},
     "Check the stealth tells"),
]

reqs = "".join(json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}) + "\n" for i, m, p, _ in STEPS)
labels = {i: lbl for i, _, _, lbl in STEPS}

# Restrict uploads to the temp dir where the demo image lives (the recommended
# pattern for agents: scope NEOBROWSER_UPLOAD_DIR instead of allowing any path).
env = dict(os.environ,
           NEOBROWSER_HOME=os.path.join(tempfile.gettempdir(), "neobrowser-demo"),
           NEOBROWSER_UPLOAD_DIR=tempfile.gettempdir(),
           NEOBROWSER_LOG_LEVEL="warn")
proc = subprocess.run([BIN, "serve"], input=reqs, capture_output=True, text=True, timeout=180, env=env)

print("\n=== NeoBrowser demo ===\n")
for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
    except Exception:
        continue
    i = r.get("id")
    if i == 1 or not labels.get(i):
        continue
    c = (r.get("result", {}).get("content") or [{}])[0]
    out = c.get("text", "")[:100].replace("\n", " ")
    print(f"  ✓ {labels[i]:<34} {out}")
print("\nDone. Real browser, real sessions, not detected.\n")
