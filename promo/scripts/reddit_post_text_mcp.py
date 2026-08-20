#!/usr/bin/env python3
"""
Post a text + link showcase to r/mcp using neobrowser (no media upload).

Usage:
    python3 promo/scripts/reddit_post_text_mcp.py [--confirm]
"""
import json, os, subprocess, sys

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
CONFIRM = "--confirm" in sys.argv

TITLE = "Showcase: NeoBrowser driving real Chrome for login + upload + bot detection"
BODY = """Most AI browser tools hand the agent a fresh headless profile. It works for public pages, but fails the moment a site needs a real session, a cookie, or a trusted fingerprint.

I built NeoBrowser to drive the user's real Google Chrome via CDP instead. The agent inherits existing cookies, fingerprint, and trust state.

This demo shows one continuous take:
- Real login form
- Real file upload through the native picker
- Passing bot.sannysoft with a genuine fingerprint (no spoofed WebGL or UA)

GIF: https://github.com/pitiflautico/neobrowser/raw/main/promo/assets/neobrowser-demo-2026-08-20.gif

Honest trade-off: ~3-4s latency per action vs a headless browser, but tasks behind real sessions actually complete.

Repo: https://github.com/pitiflautico/neobrowser

Happy to answer questions or get roasted on the architecture."""

set_values_js = f"""
const title = document.querySelector('textarea[name=\"title\"]');
const text = document.querySelector('textarea[name=\"text\"]');
title.value = {json.dumps(TITLE)};
text.value = {json.dumps(BODY)};
title.dispatchEvent(new Event('input', {{ bubbles: true }}));
text.dispatchEvent(new Event('input', {{ bubbles: true }}));
return 'values set';
"""

submit_js = """
const btn = document.querySelector('button[value="form"]');
if (btn) { btn.click(); return 'clicked'; }
return 'button not found';
"""

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://old.reddit.com/r/mcp/submit", "wait_s": 4}}),
    (3, "tools/call", {"name": "js", "arguments": {"code": set_values_js}}),
]

if CONFIRM:
    STEPS += [
        (4, "tools/call", {"name": "js", "arguments": {"code": submit_js}}),
        (5, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
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
)

print(f"[reddit_post_text_mcp] attach port={ATTACH} confirm={CONFIRM}")
proc = subprocess.run(
    [BIN, "serve"],
    input=reqs,
    capture_output=True,
    text=True,
    timeout=120,
    env=env,
)

for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
        i = r.get("id")
        if i == 3:
            content = (r.get("result", {}).get("content") or [{}])[0].get("text", "")
            print(f"[values] {content[:100]}")
        elif i == 5:
            content = (r.get("result", {}).get("content") or [{}])[0].get("text", "")
            print(f"[after submit] chars={len(content)} snippet: {content[:300].replace(chr(10), ' ')}")
    except Exception:
        continue

if not CONFIRM:
    print("Dry run complete. Run with --confirm to submit.")
else:
    print("Post submitted.")
