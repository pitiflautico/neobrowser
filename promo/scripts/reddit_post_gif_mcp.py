#!/usr/bin/env python3
"""
Post the NeoBrowser demo GIF to r/mcp using neobrowser.

Usage:
    python3 promo/scripts/reddit_post_gif_mcp.py [--confirm]

Without --confirm it stops before clicking Submit.
"""
import json, os, subprocess, sys

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
CONFIRM = "--confirm" in sys.argv
GIF_PATH = os.path.abspath("promo/assets/neobrowser-demo-2026-08-20.gif")

TITLE = "Showcase: NeoBrowser driving real Chrome for login + upload + bot detection (GIF)"
BODY = """Most AI browser tools hand the agent a fresh headless profile. It works for public pages, but fails the moment a site needs a real session, a cookie, or a trusted fingerprint.

I built NeoBrowser to drive the user's real Google Chrome via CDP instead. The agent inherits existing cookies, fingerprint, and trust state.

This GIF is one continuous take:
- Real login form
- Real file upload through the native picker
- Passing bot.sannysoft with a genuine fingerprint (no spoofed WebGL or UA)

Honest trade-off: ~3-4s latency per action vs a headless browser, but tasks behind real sessions actually complete.

Repo: https://github.com/pitiflautico/neobrowser

Happy to answer questions or get roasted on the architecture."""

focus_title_js = "document.querySelector('textarea[name=\"title\"]').focus(); document.querySelector('textarea[name=\"title\"]').click();"
focus_text_js = "document.querySelector('textarea[name=\"text\"]').focus(); document.querySelector('textarea[name=\"text\"]').click();"

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://old.reddit.com/r/mcp/submit", "wait_s": 4}}),
    (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
]

if CONFIRM:
    STEPS += [
        (4, "tools/call", {"name": "js", "arguments": {"code": focus_title_js}}),
        (5, "tools/call", {"name": "type", "arguments": {"text": TITLE, "budget_s": 2}}),
        (6, "tools/call", {"name": "js", "arguments": {"code": focus_text_js}}),
        (7, "tools/call", {"name": "type", "arguments": {"text": BODY, "budget_s": 4}}),
        (8, "tools/call", {"name": "upload", "arguments": {"selector": "input[type=\"file\"]", "files": [GIF_PATH]}}),
        (9, "tools/call", {"name": "submit", "arguments": {"selector": "button[value=\"form\"]", "wait_s": 6}}),
        (10, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
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

print(f"[reddit_post_gif_mcp] attach port={ATTACH} confirm={CONFIRM}")
proc = subprocess.run(
    [BIN, "serve"],
    input=reqs,
    capture_output=True,
    text=True,
    timeout=240,
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
        if i in (4, 6, 8):
            print(f"[step {i}] {content[:120].replace(chr(10), ' ')}")
        elif i == 10:
            print(f"[after submit] chars={len(content)} snippet: {content[:400].replace(chr(10), ' ')}")
    except Exception:
        continue

if not CONFIRM:
    print("Dry run complete. Run with --confirm to submit.")
else:
    print("Post submitted (verify in /user/Pitiflautico2/submitted).")
