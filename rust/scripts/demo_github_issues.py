#!/usr/bin/env python3
"""
NeoBrowser adapter demo — list open issues on a GitHub repo.

This is the kind of small, reusable task that is painful with a fresh
headless browser (it sees the public page, not *your* notifications or
private repos) and trivial when the agent drives your real Chrome.

Usage:
    python3 rust/scripts/demo_github_issues.py [owner/repo] [path-to-neobrowser-binary]

Defaults:
    owner/repo = pitiflautico/neobrowser
    binary     = rust/target/release/neobrowser
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "..", "target", "release", "neobrowser")
REPO = sys.argv[1] if len(sys.argv) > 1 else "pitiflautico/neobrowser"
URL = f"https://github.com/{REPO}/issues"

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": URL, "wait_s": 3}}),
    (3, "tools/call", {"name": "js", "arguments": {"code": """
        const items = Array.from(document.querySelectorAll('[data-testid="issue-title"]'));
        return JSON.stringify(items.slice(0, 5).map(el => ({
            title: el.textContent.trim(),
            href: el.closest('a')?.href || null
        })));
    """}}),
]

reqs = "".join(
    json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": p}) + "\n"
    for i, m, p in STEPS
)

env = dict(
    os.environ,
    NEOBROWSER_HOME=os.path.join(tempfile.gettempdir(), "neobrowser-demo-github"),
    NEOBROWSER_LOG_LEVEL="warn",
)

proc = subprocess.run([BIN, "serve"], input=reqs, capture_output=True, text=True, timeout=120, env=env)

print(f"\n=== NeoBrowser GitHub adapter demo: {REPO} ===\n")
for line in proc.stdout.splitlines():
    try:
        r = json.loads(line)
    except Exception:
        continue
    if r.get("id") != 3:
        continue
    content = r.get("result", {}).get("content") or [{}]
    text = content[0].get("text", "")
    try:
        issues = json.loads(text)
    except Exception:
        issues = []
    if not issues:
        print("  No issues found (GitHub may have rendered a different UI).")
    for issue in issues:
        title = issue.get("title", "")
        href = issue.get("href", "")
        print(f"  • {title}")
        if href:
            print(f"    {href}")
print("\nDone.\n")
