#!/usr/bin/env python3
"""
Navigate r/mcp with NeoBrowser, list recent threads, and optionally post a value-first comment.

Usage:
    python3 promo/scripts/reddit_value_comment.py [--post <thread_url>]

Without --post it just lists threads. With --post it attempts to add a comment.
"""
import json, os, subprocess, sys

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
POST_URL = sys.argv[2] if len(sys.argv) > 2 and sys.argv[1] == "--post" else None

if POST_URL:
    COMMENT = """\
I’ve been iterating on a real-Chrome MCP server for a few months and the thing that surprised me most is how fast you hit the "trust" wall.

A fresh headless profile can click and type just fine, but the moment a site wants a session, a cookie, or just a fingerprint that isn’t sterile, the model spends all its context budget on login flows instead of the actual task. Driving the user’s real Chrome (with opt-in, domain-scoped cookie injection) removes that friction, at the cost of ~3-4s more latency per action.

The honest trade-off seems to be:
- Stateless scraping / public pages → headless is faster and simpler.
- Anything behind a real login or upload → real Chrome wins, because the agent starts where the user already is.

Has anyone else benchmarked real-session vs headless for agent tasks? I’d love to compare numbers/methodology.
""".strip()
    STEPS = [
        (1, "initialize", {}),
        (2, "tools/call", {"name": "navigate", "arguments": {"url": POST_URL, "wait_s": 4}}),
        (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 4000}}),
        (4, "tools/call", {"name": "find_and_click", "arguments": {"text": "comment", "wait_s": 2}}),
        (5, "tools/call", {"name": "type", "arguments": {"selector": "textarea", "value": COMMENT, "human": True, "wait_s": 2}}),
        (6, "tools/call", {"name": "find_and_click", "arguments": {"text": "save", "wait_s": 2}}),
        (7, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
    ]
else:
    STEPS = [
        (1, "initialize", {}),
        (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://old.reddit.com/r/mcp/new/", "wait_s": 4}}),
        (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 6000}}),
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
        if i is None or i == 1:
            continue
        result = r.get("result", {})
        content = (result.get("content") or [{}])[0].get("text", "")
        if i == 3:
            print(f"[observe] chars={len(content)}")
            print(content[:5000])
        elif i == 7:
            print(f"[after comment] chars={len(content)} snippet: {content[:300].replace(chr(10), ' ')}")
    except Exception:
        continue
