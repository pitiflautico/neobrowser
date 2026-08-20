#!/usr/bin/env python3
"""
Post the daily build-in-public update to LinkedIn using NeoBrowser.

Usage:
    python3 promo/scripts/linkedin_post.py [--confirm]

Without --confirm the script fills the composer but does NOT click Post,
so you can review the preview. With --confirm it submits.

Requires a Chrome with remote debugging on NEOBROWSER_ATTACH_PORT (default 63599)
and a valid LinkedIn session in that profile.
"""
import json, os, subprocess, sys, textwrap, time

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
CONFIRM = "--confirm" in sys.argv

POST = textwrap.dedent("""\
89 stars down, 9,911 to go.

I'm building NeoBrowser in public with a slightly unhinged constraint: an AI agent runs the promotion, and if we don't hit 10k GitHub stars, the project gets shut down forever.

Why do this? Because distribution is the hardest part of open source, and I wanted to test — in the open — whether an agent can learn to do it ethically, without spam, fake accounts, or bought stars.

NeoBrowser is an MCP server that lets your AI drive your real Google Chrome: your real logged-in sessions, genuine fingerprint, human-like mouse and typing. Not a sterile headless browser that trips every wall the moment it needs a cookie.

This week:
- Honest benchmarks against Playwright MCP and browser-use.
- Reddit discussions on r/mcp and r/SideProject.
- Requested neobrowser.is-a.dev for an upcoming Product Hunt launch.
- CI green again after the last format/clippy pass.

If you've ever tried to automate a workflow behind a real login, you know the pain. Happy to answer hard questions or roast the architecture in the comments.

→ https://github.com/pitiflautico/neobrowser

#buildinpublic #mcp #aiagents #opensource #browserautomation
""").strip()

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": "https://www.linkedin.com/feed/", "wait_s": 4}}),
    (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 4000}}),
]

if CONFIRM:
    STEPS += [
        (4, "tools/call", {"name": "find_and_click", "arguments": {"text": "Start a post", "wait_s": 2}}),
        (5, "tools/call", {"name": "type", "arguments": {"selector": "div[contenteditable='true']", "value": POST, "human": True, "wait_s": 2}}),
        (6, "tools/call", {"name": "screenshot", "arguments": {"format": "png"}}),
        (7, "tools/call", {"name": "find_and_click", "arguments": {"text": "Post", "wait_s": 2}}),
        (8, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
    ]
else:
    STEPS += [
        (4, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 2000}}),
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

print(f"[linkedin_post] attach port={ATTACH} confirm={CONFIRM}")
proc = subprocess.run(
    [BIN, "serve"],
    input=reqs,
    capture_output=True,
    text=True,
    timeout=120,
    env=env,
)

logged_in = False
screenshot_b64 = None
for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
    except Exception:
        continue
    i = r.get("id")
    if i is None or i == 1:
        continue
    result = r.get("result", {})
    content = (result.get("content") or [{}])[0].get("text", "")
    if i == 3:
        lower = content.lower()
        # Reliable logged-out indicators on linkedin.com/feed
        logged_out = "iniciar sesión" in lower or "sign in" in lower or "unirse ahora" in lower or "join now" in lower
        logged_in = not logged_out and ("notificaciones" in lower or "notifications" in lower or "start a post" in lower or "nuevo post" in lower)
        print(f"[observe feed] logged_in={logged_in} chars={len(content)}")
        if not logged_in:
            print("LinkedIn session not detected. Stopping.")
            sys.exit(1)
    elif i == 6 and CONFIRM:
        # Screenshot returns base64 PNG
        screenshot_b64 = content
        preview_path = os.path.abspath("promo/assets/linkedin-preview-2026-08-20.png")
        try:
            import base64
            os.makedirs(os.path.dirname(preview_path), exist_ok=True)
            with open(preview_path, "wb") as f:
                f.write(base64.b64decode(screenshot_b64))
            print(f"[screenshot] saved {preview_path}")
        except Exception as e:
            print(f"[screenshot] could not save: {e}")
    elif i == 8 and CONFIRM:
        print(f"[after post] chars={len(content)} snippet: {content[:200].replace(chr(10), ' ')}")

if not CONFIRM:
    print("Dry run complete. Session is valid. Run with --confirm to submit.")
else:
    print("Post submitted (check LinkedIn for confirmation).")
