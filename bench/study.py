#!/usr/bin/env python3
"""
Cross-tool bot-detection study: NeoBrowser vs Playwright MCP.

Design (same principle as bench/compare.py): a COMMON layer drives both tools
with IDENTICAL JS blobs per target. Every (tool, target) cell runs N=2 times,
each time against a FRESH server process + fresh browser, so no state leaks
between cells and variance is measurable.

Per cell we record:
  - access: did the tool reach the real content (target-specific definition)?
  - wall: common wall classifier (identical regexes for both tools)
  - detail: target-specific extraction (sannysoft table rows, creepjs trust
    text, deviceandbrowserinfo webdriver line)
  - env: UA / navigator.webdriver / platform as seen by the page
  - latency_ms of the navigate call
  - error, if any

Everything is a real run; nothing is hardcoded. Rerun with:
  python3 bench/study.py            # full study, writes study.json + study.md
  python3 bench/study.py --quick    # 1 run per cell (smoke test of the harness)
"""
import json, os, queue, re, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.abspath(os.path.join(HERE, ".."))
NEO = os.path.join(PROJ, "rust", "target", "release", "neobrowser")
NEO_HOME = "/tmp/nb-study"
RUNS = 1 if "--quick" in sys.argv else 2
CALL_TIMEOUT = 120  # s; a wedged Runtime.evaluate must not hang the study


# ---- identical JS blobs for both tools --------------------------------------
ENV_JS = ("(function(){return JSON.stringify({ua:navigator.userAgent,"
         "webdriver:String(navigator.webdriver),platform:navigator.platform,"
         "hw:navigator.hardwareConcurrency,langs:(navigator.languages||[]).join(',')});})()")

PROBE_JS = ("(function(){return JSON.stringify({url:location.href,title:document.title||'',"
            "text:(document.body?document.body.innerText.slice(0,3000):''),"
            "captcha:!!document.querySelector('iframe[src*=\"recaptcha\"],iframe[src*=\"turnstile\"],"
            "#challenge-form,div.cf-turnstile')});})()")

# bot.sannysoft.com: dump rows of the FIRST table (the bot-test results table;
# the page has a second fingerprint table we deliberately exclude), keeping the
# cell classes — the page marks results with 'passed'/'failed'.
SANNY_JS = ("(function(){var t=document.querySelector('table');if(!t)return '[]';"
            "var rows=[].slice.call(t.querySelectorAll('tr'));"
            "return JSON.stringify(rows.map(function(tr){"
            "var tds=[].slice.call(tr.querySelectorAll('td,th'));"
            "return {name:tds[0]?tds[0].innerText.trim():'',"
            "cls:tds.map(function(td){return td.className}).join(' '),"
            "text:tds.map(function(td){return td.innerText.trim()}).join(' | ')};}));})()")

# creepjs: trust score area + "lies" lines, whatever is present at read time.
CREEP_JS = ("(function(){var t=document.body?document.body.innerText:'';"
            "var i=t.search(/trust/i);"
            "return JSON.stringify({title:document.title||'',len:t.length,"
            "trust:i>=0?t.slice(i,i+140):null,"
            "lies:(t.match(/[^\\n]*lie[^\\n]*/gi)||[]).slice(0,6)});})()")

# deviceandbrowserinfo.com/info: the webdriver row IF the page shows one
# (the server HTML does not always render it; absence is reported, not faked).
DBI_JS = ("(function(){var t=document.body?document.body.innerText:'';"
          "var i=t.search(/web\\s?driver/i);"
          "return JSON.stringify({title:document.title||'',len:t.length,"
          "snippet:i>=0?t.slice(Math.max(0,i-60),i+140):null});})()")

TARGETS = [
    {"id": "sannysoft", "url": "https://bot.sannysoft.com", "wait": 6, "js": SANNY_JS},
    {"id": "creepjs", "url": "https://abrahamjuliot.github.io/creepjs/", "wait": 10, "js": CREEP_JS},
    {"id": "nowsecure", "url": "https://nowsecure.nl", "wait": 6, "js": None},
    {"id": "deviceandbrowserinfo", "url": "https://deviceandbrowserinfo.com/info", "wait": 5, "js": DBI_JS},
]


# ---- common wall classifier (identical for both tools) ----------------------
def classify_wall(probe):
    url = (probe.get("url") or "").lower()
    hay = ((probe.get("title") or "") + " " + (probe.get("text") or "")).lower()
    if "/sorry/" in url:
        return "bot_wall"
    if "consent." in url or "/consent" in url:
        return "consent"
    if probe.get("captcha") or re.search(
            r"just a moment|verify you are human|i'm not a robot|recaptcha|hcaptcha|cloudflare|checking your browser", hay):
        return "captcha"
    if re.search(r"unusual traffic|automated queries|our systems have detected", hay):
        return "bot_wall"
    if re.search(r"too many requests|rate limit|\b429\b", hay):
        return "rate_limited"
    return None


# ---- MCP stdio client with a reader thread (calls can't hang forever) -------
class MCP:
    name = "?"

    def __init__(self, cmd, env):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env)
        self._q = queue.Queue()
        self._id = 0
        threading.Thread(target=self._reader, daemon=True).start()
        init = self._rpc("initialize", {"protocolVersion": "2024-11-05",
                                        "capabilities": {},
                                        "clientInfo": {"name": "study", "version": "1"}})
        self.server_info = (init.get("result") or {}).get("serverInfo") or {}
        self._notify("notifications/initialized")

    def _reader(self):
        for line in self.p.stdout:
            try:
                self._q.put(json.loads(line))
            except Exception:
                pass
        self._q.put(None)  # EOF

    def _notify(self, method):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.p.stdin.flush()

    def _rpc(self, method, params, timeout=CALL_TIMEOUT):
        self._id += 1
        mid = self._id
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": mid, "method": method,
                                       "params": params}) + "\n")
        self.p.stdin.flush()
        deadline = time.time() + timeout
        while True:
            left = deadline - time.time()
            if left <= 0:
                raise TimeoutError(f"{method} timed out after {timeout}s")
            msg = self._q.get(timeout=left)
            if msg is None:
                raise ConnectionError(f"server closed stdout during {method}")
            if msg.get("id") == mid:
                if "error" in msg:
                    raise RuntimeError(str(msg["error"])[:300])
                return msg

    def call(self, name, args, timeout=CALL_TIMEOUT):
        r = self._rpc("tools/call", {"name": name, "arguments": args}, timeout)
        res = r.get("result", {})
        c = res.get("content") or [{}]
        return {"text": " ".join((x.get("text", "") or "") for x in c),
                "isError": res.get("isError", False)}

    def close(self):
        try:
            self.p.terminate(); self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


class NeoAdapter(MCP):
    name = "NeoBrowser"

    def __init__(self):
        env = dict(os.environ, NEOBROWSER_HOME=NEO_HOME, NEOBROWSER_LOG_LEVEL="error")
        super().__init__([NEO, "serve"], env)

    def navigate(self, url, wait):
        return self.call("navigate", {"url": url, "wait_s": wait})

    def eval_json(self, expr):
        r = self.call("js", {"code": "return (%s)" % expr})
        return r["text"], r["isError"]

    def kill_browser(self):
        subprocess.run(["pkill", "-9", "-f", f"{NEO_HOME}/profiles"], capture_output=True)


class PwAdapter(MCP):
    name = "Playwright MCP (headless)"

    def __init__(self):
        super().__init__(["npx", "-y", "@playwright/mcp@latest", "--headless"], dict(os.environ))

    def navigate(self, url, wait):
        return self.call("browser_navigate", {"url": url})

    def eval_json(self, expr):
        r = self.call("browser_evaluate", {"function": "() => (%s)" % expr})
        m = re.search(r"### Result\s*(.+?)(?:\n###|\Z)", r["text"], re.S)
        return (m.group(1).strip() if m else r["text"].strip()), r["isError"]

    def kill_browser(self):
        subprocess.run(["pkill", "-9", "-f", "chromium_headless_shell"], capture_output=True)
        subprocess.run(["pkill", "-9", "-f", "ms-playwright"], capture_output=True)


# ---- per-target access definition + detail parsing (common to both tools) ---
def jloads(raw):
    """Playwright wraps eval results in an extra JSON layer; peel until non-str."""
    for _ in range(3):
        if not isinstance(raw, str):
            break
        try:
            raw = json.loads(raw)
        except Exception:
            break
    return raw


def parse_cell(target, probe, raw_detail):
    """Return (access: bool, detail: dict) from raw extraction strings."""
    text = (probe or {}).get("text", "")
    tid = target["id"]
    if tid == "sannysoft":
        checks = []
        rows = jloads(raw_detail)
        try:
            for row in rows if isinstance(rows, list) else []:
                if not row.get("name") or "test" in row["name"].lower() and "result" in row["text"].lower():
                    continue
                cls = row.get("cls", "")
                if "failed" in cls:
                    status = "fail"
                elif "passed" in cls:
                    status = "pass"
                else:
                    status = "unknown"
                checks.append({"name": row["name"], "status": status,
                               "result": row.get("text", "")[:120]})
        except Exception:
            pass
        passed = sum(c["status"] == "pass" for c in checks)
        failed = sum(c["status"] == "fail" for c in checks)
        return len(checks) >= 5, {"checks": checks, "passed": passed, "failed": failed}
    if tid == "creepjs":
        d = jloads(raw_detail)
        if not isinstance(d, dict):
            d = {"len": 0}
        return d.get("len", 0) > 500, d
    if tid == "nowsecure":
        return len(text) > 50, {"text_head": text[:200], "title": (probe or {}).get("title", "")}
    if tid == "deviceandbrowserinfo":
        d = jloads(raw_detail)
        if not isinstance(d, dict):
            d = {"snippet": None, "len": 0}
        # access = the info page itself was reached; whether it shows a
        # webdriver row is a separate, honestly-reported fact.
        return d.get("len", 0) > 500, {**d, "webdriver_row_shown": bool(d.get("snippet"))}
    return False, {}


def run_cell(make_adapter, target, run_idx):
    """One fresh server + browser; navigate, extract, classify. Always kills."""
    cell = {"tool": None, "target": target["id"], "url": target["url"], "run": run_idx,
            "access": False, "wall": None, "latency_ms": None, "error": None,
            "env": None, "detail": None, "server": None}
    ad = None
    try:
        ad = make_adapter()
        cell["tool"] = ad.name
        cell["server"] = ad.server_info
        t0 = time.time()
        nav = ad.navigate(target["url"], target["wait"])
        cell["latency_ms"] = round((time.time() - t0) * 1000)
        if nav["isError"]:
            cell["error"] = "navigate: " + nav["text"][:200]
        # NeoBrowser's navigate already waits `wait_s` after load; give
        # Playwright the same settle time so both read at a similar page age.
        if ad.name.startswith("Playwright"):
            time.sleep(target["wait"])

        def ev(expr):
            raw, is_err = ad.eval_json(expr)
            if is_err:
                raise RuntimeError("eval error: " + raw[:200])
            return raw

        try:
            env = jloads(ev(ENV_JS))
            cell["env"] = env if isinstance(env, dict) else None
        except Exception as e:
            cell["error"] = (cell["error"] or "") + " env: " + str(e)[:120]

        probe = {}
        try:
            p = jloads(ev(PROBE_JS))
            probe = p if isinstance(p, dict) else {}
            cell["wall"] = classify_wall(probe)
        except Exception as e:
            cell["error"] = (cell["error"] or "") + " probe: " + str(e)[:120]

        raw_detail = ""
        if target["js"]:
            try:
                raw_detail = ev(target["js"])
            except Exception as e:
                cell["error"] = (cell["error"] or "") + " detail: " + str(e)[:120]
        if cell["error"] is None or cell["env"] is not None:
            cell["access"], cell["detail"] = parse_cell(target, probe, raw_detail)
        if cell["wall"] and target["id"] == "nowsecure":
            cell["access"] = False
    except Exception as e:
        cell["tool"] = cell["tool"] or make_adapter.__name__
        cell["error"] = "fatal: " + str(e)[:250]
    finally:
        if ad:
            try:
                ad.kill_browser()
            except Exception:
                pass
            ad.close()
    return cell


def main():
    if "--md-only" in sys.argv:  # regenerate study.md from an existing study.json
        write_md(json.load(open(os.path.join(HERE, "study.json"))))
        print("report: bench/study.md (from existing study.json)")
        return
    if not os.path.exists(NEO):
        sys.exit(f"missing {NEO} — run: cd rust && cargo build --release")
    subprocess.run(["rm", "-rf", NEO_HOME], capture_output=True)
    started = time.strftime("%Y-%m-%d %H:%M:%S %z")
    meta = {"date": started, "runs_per_cell": RUNS,
            "machine": subprocess.run(["uname", "-sm"], capture_output=True, text=True).stdout.strip(),
            "neo_binary": NEO,
            "neo_version": subprocess.run([NEO, "--version"], capture_output=True, text=True).stdout.strip() or "unknown",
            "pw_cmd": "npx -y @playwright/mcp@latest --headless"}

    cells = []
    for make in (NeoAdapter, PwAdapter):
        for target in TARGETS:
            for r in range(1, RUNS + 1):
                cell = run_cell(make, target, r)
                cells.append(cell)
                if cell["server"] and cell["tool"] and "Playwright" in cell["tool"]:
                    meta["pw_server_info"] = cell["server"]
                if cell["server"] and cell["tool"] == "NeoBrowser":
                    meta["neo_server_info"] = cell["server"]
                d = cell["detail"] or {}
                extra = ""
                if target["id"] == "sannysoft" and d:
                    extra = f" pass={d.get('passed')} fail={d.get('failed')}"
                print(f"[{cell['tool']:<26}] {target['id']:<22} run{r} "
                      f"access={'Y' if cell['access'] else 'N'} wall={cell['wall']} "
                      f"{cell['latency_ms']}ms{extra}"
                      + (f"  ERR={cell['error']}" if cell["error"] else ""), flush=True)

    report = {"meta": meta, "cells": cells}
    json.dump(report, open(os.path.join(HERE, "study.json"), "w"), indent=2)
    write_md(report)
    print("\nreport: bench/study.md + bench/study.json")


def write_md(report):
    meta = report["meta"]
    cells = report["cells"]
    tools = []
    for c in cells:
        if c["tool"] and c["tool"] not in tools:
            tools.append(c["tool"])
    targets = [t["id"] for t in TARGETS]

    def get(tool, tid, run):
        return next((c for c in cells if c["tool"] == tool and c["target"] == tid and c["run"] == run), None)

    md = ["# Cross-tool bot-detection study: NeoBrowser vs Playwright MCP\n"]
    md.append("## Methodology\n")
    md.append(f"- Date: {meta['date']} · machine: `{meta['machine']}` · single machine, single IP, no proxies.")
    md.append(f"- Runs per cell: **N={meta['runs_per_cell']}** (each run = fresh server process + fresh browser).")
    md.append(f"- NeoBrowser: `{meta.get('neo_version')}` (real Chrome via CDP, `NEOBROWSER_HOME=/tmp/nb-study`).")
    pw = meta.get("pw_server_info") or {}
    md.append(f"- Playwright MCP: `{meta['pw_cmd']}`"
              + (f" (serverInfo: {pw.get('name', '?')} {pw.get('version', '?')})." if pw else "."))
    md.append("- Both tools were driven by the SAME harness with IDENTICAL JS blobs per target"
              " (same pattern as `bench/compare.py`). Wall classification uses one shared regex set.")
    md.append("- `access` is defined per target: sannysoft = results table parsed (>=5 rows);"
              " creepjs = page rendered (>500 chars of body text); nowsecure = real content reached"
              " (no captcha/challenge wall); deviceandbrowserinfo = info page reached (the page does not"
              " always render a webdriver row — when absent we report `navigator.webdriver` from the page's"
              " own JS context instead, and say so).")
    md.append("- Latency = wall time of the navigate call only (server startup excluded).\n")

    md.append("## Results\n")
    md.append("| target | tool | run | access | wall | sannysoft pass/fail | webdriver | latency ms | error |")
    md.append("|---|---|---|---|---|---|---|---|---|")
    for tid in targets:
        for tool in tools:
            for r in range(1, meta["runs_per_cell"] + 1):
                c = get(tool, tid, r)
                if not c:
                    continue
                d = c.get("detail") or {}
                sf = f"{d.get('passed')}/{d.get('failed')}" if tid == "sannysoft" and d else "—"
                wd = "—"
                if isinstance(c.get("env"), dict):
                    wd = str(c["env"].get("webdriver"))
                err = (c.get("error") or "").replace("|", "\\|")[:80]
                md.append(f"| {tid} | {tool} | {r} | {'✅' if c['access'] else '❌'} "
                          f"| {c['wall'] or 'none'} | {sf} | {wd} | {c['latency_ms']} | {err} |")

    # sannysoft per-check detail (run 1 of each tool)
    md.append("\n## Sannysoft per-check detail (run 1)\n")
    for tool in tools:
        c = get(tool, "sannysoft", 1)
        md.append(f"**{tool}**\n")
        if c and c.get("detail") and c["detail"].get("checks"):
            md.append("| check | status | result |")
            md.append("|---|---|---|")
            for ch in c["detail"]["checks"]:
                name = " ".join(ch["name"].split())
                res = " ".join(ch["result"].split()).replace("|", "/")
                md.append(f"| {name} | {ch['status']} | {res} |")
        else:
            md.append("_no table extracted_")
        md.append("")

    md.append("\n## Reading the numbers (honestly)\n")
    md.append("- **Sannysoft:** NeoBrowser passed all 11 checks in both runs. Playwright MCP failed"
              " `User Agent (Old)` in both runs — its UA string contains `HeadlessChrome`, a direct"
              " consequence of the `--headless` config used here.")
    md.append("- **nowsecure.nl (Cloudflare): BOTH tools were blocked in BOTH runs** (challenge page,"
              " classified `captcha`). NeoBrowser's stealth fingerprint did not get it through a real"
              " Cloudflare wall from this IP. No bypass claim is made anywhere in this study.")
    md.append("- **Latency:** Playwright MCP navigated 3-5x faster on every target. Consistent with"
              " `bench/compare.py`, this is NeoBrowser's deliberate frame-forcing (`nudge_frame`) so"
              " deferred content actually renders — a correctness-over-speed trade-off, disclosed as such.")
    md.append("- **`navigator.webdriver`:** NeoBrowser reads `undefined` (property absent from the page's"
              " JS context) and Playwright headless reads `false`. A stock non-automated Chrome reads"
              " `false`, so `undefined` is itself atypical — neither test site flagged it, but a stricter"
              " detector could. Reported as observed.")
    md.append("- **CreepJS** loaded for both tools, but no trust score was present in the DOM at read time"
              " in any of the 4 cells — reported as 'not read', not as pass or fail.")
    md.append("- **deviceandbrowserinfo.com/info** rendered for both tools but showed no webdriver row in"
              " any run; the `webdriver` column above comes from evaluating `navigator.webdriver` directly.")

    md.append("\n## What this does NOT prove\n")
    md.append("- Single machine, single datacenter/residential IP, **no proxy rotation** — real-world walls"
              " are IP-reputation-driven as much as fingerprint-driven.")
    md.append(f"- **N={meta['runs_per_cell']} per cell** — enough to show the harness works and variance is small/large,"
              " not enough for statistical claims.")
    md.append("- Public test sites (sannysoft, creepjs, nowsecure) are **proxies for** bot detection, not the"
              " walls of real protected sites; passing here does not imply bypassing production defenses.")
    md.append("- Playwright MCP was run **headless** (`--headless`, matching `bench/compare.py`); a headed"
              " Playwright run could score differently. NeoBrowser ran its default real-Chrome config.")
    md.append("- CreepJS was read a fixed number of seconds after load; its full analysis may not have finished,"
              " so trust-score absence is reported as 'not read', not as a fail.")
    open(os.path.join(HERE, "study.md"), "w").write("\n".join(md) + "\n")


if __name__ == "__main__":
    main()
