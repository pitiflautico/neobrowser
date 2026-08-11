#!/usr/bin/env python3
"""
NeoBrowser reproducible benchmark.

Drives an MCP browser tool through a task matrix and reports task-success rate,
bot-wall rate, latency, crashes, and self-healing recovery. Ships a NeoBrowser
adapter; other tools (Playwright MCP, browser-use, manual Chrome) plug in by
implementing the same `Adapter.call()` over their own MCP/agent interface.

HONEST LIMITS
- Adversarial protections (Cloudflare / DataDome / Akamai / PerimeterX) are
  IP-reputation-sensitive and non-deterministic. Rigorous bypass-rate numbers need
  residential proxies + many runs + statistics — out of scope for a single-IP run.
  Here we measure wall DETECTION and whether content was reachable, not "bypass".
- tokens/task is an agent-loop metric (the tool server consumes none); add it via
  an LLM-driven adapter (e.g. browser-use).

Usage:
    python3 bench/run.py [path-to-neobrowser-binary]
"""
import base64, json, os, subprocess, sys, time, signal, tempfile, glob

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.abspath(os.path.join(HERE, ".."))
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(PROJ, "rust", "target", "release", "neobrowser")
HOME = "/tmp/nb-bench-home"
UPLOAD_DIR = "/tmp/nb-bench-upload"


class MCP:
    """Persistent MCP-over-stdio client for the NeoBrowser binary."""
    def __init__(self, binary, home, env_extra=None):
        env = dict(os.environ, NEOBROWSER_HOME=home, NEOBROWSER_LOG_LEVEL="error")
        if env_extra:
            env.update(env_extra)
        self.env = env
        self.p = subprocess.Popen([binary, "serve"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                  text=True, env=env, bufsize=1)
        self._id = 0
        self._rpc("initialize", {})

    def _rpc(self, method, params):
        self._id += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self._id, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        return json.loads(line) if line else {}

    def call(self, name, args):
        r = self._rpc("tools/call", {"name": name, "arguments": args})
        res = r.get("result", {})
        c = (res.get("content") or [{}])[0]
        return {"text": c.get("text", ""), "image": c.get("type") == "image", "isError": res.get("isError", False)}

    def close(self):
        try:
            self.p.stdin.close(); self.p.terminate(); self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


def contains(text, needle):
    return needle.lower() in (text or "").lower()


def run_step(mcp, step):
    tool, args = step["tool"], dict(step.get("args", {}))
    if tool == "expect":
        r = mcp.call("read", {"selector": args.get("selector", "body")})
        return contains(r["text"], args["contains"]), r
    r = mcp.call(tool, args)
    if r["isError"]:
        return False, r
    if "expect_contains" in step:
        return contains(r["text"], step["expect_contains"]), r
    return True, r


def kill_chrome(home):
    """Force-kill the isolated Chrome so we can test self-healing recovery."""
    subprocess.run(["pkill", "-9", "-f", f"{home}/profiles"], capture_output=True)


def special_multitab(mcp):
    mcp.call("navigate", {"url": "https://example.com", "wait_s": 1})
    mcp.call("new_tab", {})
    mcp.call("navigate", {"url": "https://httpbin.org/html", "wait_s": 1})
    lst = mcp.call("list_tabs", {})
    ok = False
    try:
        ok = json.loads(lst["text"]).get("tabs") and len(json.loads(lst["text"])["tabs"]) == 2
    except Exception:
        pass
    mcp.call("close_tab", {"index": 1})
    return bool(ok)


def special_recovery(mcp, home):
    mcp.call("navigate", {"url": "https://example.com", "wait_s": 1})
    kill_chrome(home)
    time.sleep(1)
    r = mcp.call("navigate", {"url": "https://example.com", "wait_s": 2})
    return contains(r["text"], "navigated to") and not r["isError"]


def special_persistence(mcp):
    mcp.call("navigate", {"url": "https://example.com", "wait_s": 1})
    mcp.call("js", {"code": 'document.cookie="benchp=1; path=/"; return document.cookie'})
    saved = mcp.call("save_cookies", {})
    restored = mcp.call("restore_cookies", {})
    return contains(saved["text"], "saved") and ("restored 1" in restored["text"].lower() or "restored" in restored["text"].lower())


def main():
    os.makedirs(UPLOAD_DIR, exist_ok=True)
    img = os.path.join(UPLOAD_DIR, "bench.png")
    open(img, "wb").write(base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="))
    subprocess.run(["rm", "-rf", HOME], capture_output=True)

    tasks = json.load(open(os.path.join(HERE, "tasks.json")))
    for t in tasks:  # inject the bench image path
        for s in t.get("steps", []):
            for k, v in list(s.get("args", {}).items()):
                if v == ["__BENCH_IMG__"]:
                    s["args"][k] = [img]

    mcp = MCP(BIN, HOME, {"NEOBROWSER_UPLOAD_DIR": UPLOAD_DIR})
    results = []
    for t in tasks:
        t0 = time.time()
        wall = None
        error = None
        try:
            special = t.get("special")
            if special == "multitab":
                success = special_multitab(mcp)
            elif special == "recovery":
                success = special_recovery(mcp, HOME)
            elif special == "persistence":
                success = special_persistence(mcp)
            else:
                nav = mcp.call("navigate", {"url": t["url"], "wait_s": 2.5})
                if "⚠️" in nav["text"]:
                    wall = nav["text"].split("⚠️", 1)[1].split(":", 1)[0].strip()
                if t.get("expect_wall"):
                    success = wall is not None
                elif not t.get("steps"):
                    # pure reachability: navigated and no hard error
                    success = contains(nav["text"], "navigated to") and not nav["isError"]
                else:
                    success = True
                    for s in t["steps"]:
                        ok, _ = run_step(mcp, s)
                        if not ok:
                            success = False
                            break
        except Exception as e:
            success = False
            error = str(e)[:120]
        results.append({
            "id": t["id"], "category": t["category"], "success": success,
            "wall": wall, "latency_ms": round((time.time() - t0) * 1000), "error": error,
        })
        print(f"  {'✓' if success else '✗'} {t['id']:<22} {t['category']:<18} {results[-1]['latency_ms']:>6}ms"
              + (f"  wall={wall}" if wall else "") + (f"  ERR={error}" if error else ""))
    mcp.close()

    n = len(results)
    ok = sum(r["success"] for r in results)
    walls = [r for r in results if r["wall"]]
    crashes = [r for r in results if r["error"]]
    avg_lat = round(sum(r["latency_ms"] for r in results) / n)
    report = {
        "tool": "neobrowser", "binary": BIN, "tasks": n,
        "task_success_rate": round(ok / n, 3),
        "bot_wall_rate": round(len(walls) / n, 3),
        "avg_latency_ms": avg_lat,
        "crashes": len(crashes),
        "recovery_ok": next((r["success"] for r in results if r["category"] == "crash-recovery"), None),
        "results": results,
    }
    json.dump(report, open(os.path.join(HERE, "report.json"), "w"), indent=2)

    md = [f"# NeoBrowser benchmark — first pass\n",
          f"Tool: `neobrowser` · tasks: {n} · single IP, single run (see honest limits in run.py).\n",
          "| metric | value |", "|---|---|",
          f"| task success rate | **{ok}/{n} = {report['task_success_rate']*100:.0f}%** |",
          f"| bot-wall detection rate | {len(walls)}/{n} |",
          f"| avg latency / task | {avg_lat} ms |",
          f"| crashes | {len(crashes)} |",
          f"| crash-recovery (self-heal) | {'PASS' if report['recovery_ok'] else 'FAIL'} |",
          "\n## Per task\n", "| task | category | result | latency | wall |", "|---|---|---|---|---|"]
    for r in results:
        md.append(f"| {r['id']} | {r['category']} | {'✓' if r['success'] else '✗'} | {r['latency_ms']}ms | {r['wall'] or ''} |")
    open(os.path.join(HERE, "report.md"), "w").write("\n".join(md) + "\n")
    print(f"\n{ok}/{n} tasks passed · avg {avg_lat}ms · walls {len(walls)} · crashes {len(crashes)} · "
          f"recovery {'PASS' if report['recovery_ok'] else 'FAIL'}")
    print(f"report: bench/report.md + bench/report.json")


if __name__ == "__main__":
    main()
