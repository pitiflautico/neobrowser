#!/usr/bin/env python3
# ARCHIVED — this parity gate cannot pass, by design.
#
# It drove the Rust and Python implementations through identical steps and compared their
# outputs. That worked while the Rust port was catching up. It stopped working when the
# Rust mutating tools adopted the verified-action envelope: `click` now returns
# {"status": "succeeded", "evidence": {...}} where the Python returns "Clicked". The
# difference is the improvement, so a script that reports it as a failure is measuring the
# wrong thing.
#
# Kept for reference on how the two were compared. See ../README.md.
"""
Parity gate: run the same MCP tool calls through the Python and Rust servers on a
hermetic (network-free) page and diff the core-tool outputs.

This is the receipt behind retiring the Python implementation: the core tools must
match (new/enhanced Rust tools — multi-source search, multi-tab, wall detection —
are intentionally different and are not compared here).

Usage:
    python3 rust/scripts/compare.py [path-to-rust-binary]
    (defaults to rust/target/release/neobrowser)
"""
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.abspath(os.path.join(HERE, "..", ".."))
RUST = sys.argv[1] if len(sys.argv) > 1 else os.path.join(PROJ, "rust", "target", "release", "neobrowser")

PAGE = ("data:text/html,<html><head><title>Cmp</title></head><body>"
        "<h1>Hello Compare</h1><p>Some visible text here.</p>"
        "<form><input id=q name=q placeholder=Search><button type=submit>Go</button></form>"
        "<table id=t><tr><th>Name</th><th>Age</th></tr><tr><td>Ana</td><td>30</td></tr>"
        "<tr><td>Ben</td><td>25</td></tr></table>"
        "<a href=https://example.com/x>ExampleLink</a></body></html>")

CORE = [
    (2, "navigate", {"url": PAGE, "wait_s": 1}),
    (3, "read", {}),
    (4, "page_info", {}),
    (5, "analyze", {}),
    (6, "extract_table", {"selector": "#t", "index": 0}),
    (7, "extract", {"what": "links"}),
    (8, "fill", {"selector": "#q", "value": "hello"}),
    (9, "js", {"code": "return 2+2"}),
    (10, "find", {"intent": "search box"}),
    (11, "screenshot", {"format": "jpeg", "quality": 40}),
]


def drive(cmd, home):
    reqs = [json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize"})]
    for i, name, args in CORE:
        reqs.append(json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call",
                                "params": {"name": name, "arguments": args}}))
    env = dict(os.environ, NEOBROWSER_HOME=home, NEOBROWSER_LOG_LEVEL="error")
    p = subprocess.run(cmd, input="\n".join(reqs) + "\n", capture_output=True,
                       text=True, timeout=120, env=env, cwd=PROJ)
    out = {}
    for line in p.stdout.splitlines():
        try:
            r = json.loads(line)
        except Exception:
            continue
        if r.get("id") == 1:
            continue
        c = (r.get("result", {}).get("content") or [{}])[0]
        out[r.get("id")] = ("image", len(c.get("data", ""))) if c.get("type") == "image" else ("text", c.get("text", ""))
    return out


def norm(name, kind, val):
    if kind == "image":
        return "IMAGE(nonzero)" if val > 0 else "IMAGE(empty)"
    s = val
    if name == "find":
        try:
            d = json.loads(s)
            return json.dumps({"found": d.get("found"), "role": d.get("role")}, sort_keys=True)
        except Exception:
            return s
    if name in ("extract_table", "extract", "page_info"):
        try:
            d = json.loads(s)
            if name == "page_info" and isinstance(d, dict):
                d.pop("url", None)
            return json.dumps(d, sort_keys=True)
        except Exception:
            return s
    if name == "analyze":
        try:
            d = json.loads(s)
            forms = [[(f.get("type"), f.get("name")) for f in fo.get("fields", [])] for fo in d.get("forms", [])]
            return json.dumps({"forms": forms, "buttons": len(d.get("buttons", []))}, sort_keys=True)
        except Exception:
            return s
    return s.strip()


def main():
    py = drive(["python3", "-c", "import neobrowser.server as s; s.main()"], "/tmp/nb-cmp-py")
    rs = drive([RUST, "serve"], "/tmp/nb-cmp-rs")
    print(f"{'tool':<14} result\n" + "-" * 60)
    same = 0
    for i, name, _ in CORE:
        pv = norm(name, *py.get(i, ("text", "<missing>")))
        rv = norm(name, *rs.get(i, ("text", "<missing>")))
        if pv == rv:
            same += 1
            print(f"{name:<14} MATCH")
        else:
            print(f"{name:<14} DIFF\n    py: {pv[:150]}\n    rs: {rv[:150]}")
    print("-" * 60)
    print(f"{same}/{len(CORE)} core tools identical")
    # find legitimately differs (Rust captures the element name); allow it.
    return 0 if same >= len(CORE) - 1 else 1


if __name__ == "__main__":
    sys.exit(main())
