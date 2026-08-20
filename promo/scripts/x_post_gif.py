#!/usr/bin/env python3
"""
Post the NeoBrowser demo GIF to X using neobrowser.

Usage:
    python3 promo/scripts/x_post_gif.py [--confirm]
"""
import json, os, subprocess, sys

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
CONFIRM = "--confirm" in sys.argv
GIF_PATH = os.path.abspath("promo/assets/neobrowser-demo-2026-08-20.gif")

TEXT = """Real login. Real upload. Real bot-detector pass.

NeoBrowser drives your actual Chrome instead of a sterile headless browser. The trade-off is ~3s more latency; the win is tasks that headless can't even start.

Repo in bio."""

focus_editor_js = """
const editor = document.querySelector('[data-testid="tweetTextarea_0"], div[contenteditable="true"], [role="textbox"]');
if (editor) { editor.focus(); editor.click(); return 'editor focused'; }
return 'editor not found';
"""

upload_js = """
const input = document.querySelector('input[type="file"]');
if (input) { input.dispatchEvent(new MouseEvent('click', {bubbles:true})); return 'file input clicked'; }
return 'no file input';
"""

post_js = """
const btn = Array.from(document.querySelectorAll('button')).find(b => b.innerText.toLowerCase().includes('post') && !b.disabled);
if (btn) { btn.click(); return 'post clicked'; }
return 'post button not found';
"""

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://x.com/compose/post", "wait_s": 4}}),
    (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
]

if CONFIRM:
    STEPS += [
        (4, "tools/call", {"name": "js", "arguments": {"code": focus_editor_js}}),
        (5, "tools/call", {"name": "type", "arguments": {"text": TEXT, "budget_s": 3}}),
        (6, "tools/call", {"name": "upload", "arguments": {"selector": "input[type=\"file\"]", "files": [GIF_PATH]}}),
        (7, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 1500}}),
        (8, "tools/call", {"name": "js", "arguments": {"code": post_js}}),
        (9, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 1500}}),
    ]

reqs = "".join(
    json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}) + "\n"
    for i, m, p in STEPS
)

env = dict(
    os.environ,
    NEOBROWSER_ATTACH_PORT=ATTACH,
    NEOBROWSER_LOG_LEVEL="warn",
    NEOBROWSER_HOME=os.path.join(os.path.expanduser("~"), ".neobrowser-promo"),
    NEOBROWSER_UPLOAD_DIR=os.path.dirname(GIF_PATH),
)

print(f"[x_post_gif] attach port={ATTACH} confirm={CONFIRM}")
proc = subprocess.run(
    [BIN, "serve"],
    input=reqs,
    capture_output=True,
    text=True,
    timeout=180,
    env=env,
)

for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
        i = r.get("id")
        if i is None or i == 1:
            continue
        result = r.get("result", {})
        content = (result.get("content") or [{}])[0].get("text", "")
        if i in (4, 5, 6, 8):
            print(f"[step {i}] {content[:120].replace(chr(10), ' ')}")
        elif i in (7, 9):
            print(f"[observe {i}] chars={len(content)} snippet: {content[:250].replace(chr(10), ' ')}")
    except Exception:
        continue

if not CONFIRM:
    print("Dry run complete. Run with --confirm to post.")
else:
    print("Post action completed.")
