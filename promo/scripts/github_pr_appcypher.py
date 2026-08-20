#!/usr/bin/env python3
"""
Create the NeoBrowser PR on appcypher/awesome-mcp-servers via GitHub web UI using neobrowser.
"""
import json, os, subprocess, sys

BIN = os.path.join("rust", "target", "release", "neobrowser")
ATTACH = os.environ.get("NEOBROWSER_ATTACH_PORT", "63599")
COMPARE_URL = "https://github.com/appcypher/awesome-mcp-servers/compare/main...pitiflautico:awesome-mcp-servers-1:add-neobrowser-2026-08-20?expand=1"

STEPS = [
    (1, "initialize", {}),
    (2, "tools/call", {"name": "navigate", "arguments": {"url": COMPARE_URL, "wait_s": 5}}),
    (3, "tools/call", {"name": "observe", "arguments": {"mode": "visible", "budget_chars": 4000}}),
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

print(f"[github_pr_appcypher] opening {COMPARE_URL}")
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
        if r.get("id") == 3:
            content = (r.get("result", {}).get("content") or [{}])[0].get("text", "")
            print(content[:3000])
            lower = content.lower()
            if "create pull request" in lower or "open a pull request" in lower:
                print("\n[OK] PR form loaded. Review and click 'Create pull request' manually, or rerun with --confirm.")
            elif "compare and review" in lower or "there isn" in lower:
                print("\n[BLOCKED] GitHub did not load the PR form. Possible auth or branch issue.")
            else:
                print("\n[INFO] PR form state uncertain; check output above.")
    except Exception:
        continue
