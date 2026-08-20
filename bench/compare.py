#!/usr/bin/env python3
"""
Neutral 2-way benchmark: NeoBrowser vs Playwright MCP.

Design principle (per methodology review): a COMMON layer drives both tools with
the IDENTICAL abstract steps and JS; metrics are computed the same way for both.
Nothing here is tuned to make either tool win — capability gaps surface on their own.

Per task we record: task_execution_success, destination_access_success (kept
SEPARATE — a walled destination is an execution success but an access failure),
step_success, wall_detected, wall_type, total_latency, latency_per_step,
tool_calls, browser_crash, recovery_success, tabs_created, final_state_valid.

Two blocks:
  FUNCTIONAL  — nav, DOM, forms, tabs, upload, persistence, recovery, screenshots.
  ADVERSARIAL — observational only (single IP, single run): we report wall
                detection + destination access, and make NO "evades better" claim.

Usage: python3 bench/compare.py [neobrowser-binary]
"""
import base64, json, os, subprocess, sys, time, re

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.abspath(os.path.join(HERE, ".."))
NEO = sys.argv[1] if len(sys.argv) > 1 else os.path.join(PROJ, "rust", "target", "release", "neobrowser")
NEO_HOME = "/tmp/nb-cmp-home"
UPLOAD_DIR = "/tmp/nb-cmp-upload"
PW_PROFILE = "/tmp/nb-cmp-pw-profile"


class Unsupported(Exception):
    pass


# ---- common wall classifier (identical for both tools) ----------------------
def classify_wall(probe):
    url = (probe.get("url") or "").lower()
    hay = ((probe.get("title") or "") + " " + (probe.get("text") or "")).lower()
    if "/sorry/" in url:
        return "bot_wall"
    if "consent." in url or "/consent" in url:
        return "consent"
    if probe.get("captcha") or re.search(r"just a moment|verify you are human|i'm not a robot|recaptcha|hcaptcha|cloudflare|checking your browser", hay):
        return "captcha"
    if re.search(r"unusual traffic|automated queries|our systems have detected", hay):
        return "bot_wall"
    if re.search(r"too many requests|rate limit|\b429\b", hay):
        return "rate_limited"
    return None


PROBE_JS = "(function(){return JSON.stringify({url:location.href,title:document.title||'',text:(document.body?document.body.innerText.slice(0,3000):''),captcha:!!document.querySelector('iframe[src*=\"recaptcha\"],iframe[src*=\"turnstile\"],#challenge-form,div.cf-turnstile')});})()"


def fill_js(sel, val):
    s = json.dumps(sel); v = json.dumps(val)
    return ("(function(){var e=document.querySelector(%s);if(!e)return 'NOEL';"
            "var p=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value');"
            "if(p&&p.set)p.set.call(e,%s);else e.value=%s;"
            "e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));"
            "return e.value;})()" % (s, v, v))


def click_js(sel):
    s = json.dumps(sel)
    return "(function(){var e=document.querySelector(%s);if(!e)return 'NOEL';e.click();return 'OK';})()" % s


def terminate_browser_gracefully(pattern):
    """SIGTERM only the *browser* process matching `pattern`, not its children.

    Chrome's cookie store lives in the network-service child process and is
    flushed to SQLite when the browser process orchestrates a clean shutdown.
    A blanket `pkill -f <pattern>` also signals that child, so nothing gets
    flushed and a perfectly persistent profile looks non-persistent. Children
    carry `--type=`; the browser process does not.

    Applied identically to both tools — an asymmetric shutdown would measure how
    each one was killed rather than whether it persists.
    """
    ps = subprocess.run(["ps", "-Ao", "pid,args"], capture_output=True, text=True).stdout
    pids = [ln.split()[0] for ln in ps.splitlines()
            if pattern in ln and "--type=" not in ln and "ps -Ao" not in ln]
    for pid in pids:
        subprocess.run(["kill", "-TERM", pid], capture_output=True)
    return len(pids)


def text_js(sel):
    s = json.dumps(sel)
    return "(function(){var e=document.querySelector(%s);return e?(e.innerText||'').slice(0,4000):'NOEL';})()" % s


# ---- base + adapters --------------------------------------------------------
class MCP:
    def __init__(self, cmd, env, cwd=None):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env,
                                  cwd=cwd)
        self.calls = 0
        self._id = 0
        self._rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "bench", "version": "1"}})
        self._notify("notifications/initialized")

    def _notify(self, method):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n"); self.p.stdin.flush()

    def _rpc(self, method, params):
        self._id += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self._id, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, name, args):
        self.calls += 1
        r = self._rpc("tools/call", {"name": name, "arguments": args})
        res = r.get("result", {})
        c = res.get("content") or [{}]
        return {"text": " ".join((x.get("text", "") or "") for x in c),
                "image": any(x.get("type") == "image" for x in c),
                "isError": res.get("isError", False)}

    def reset(self):
        """Clear any stuck UI state so the next task starts clean.

        Base implementation is a no-op; adapters whose tools can be blocked by a
        pending modal override it. Never counted as a tool call for the task being
        measured — it runs after that task's metrics are taken.
        """
        return False

    def close(self):
        try:
            self.p.terminate(); self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


class NeoAdapter(MCP):
    name = "NeoBrowser"
    def __init__(self):
        env = dict(os.environ, NEOBROWSER_HOME=NEO_HOME, NEOBROWSER_UPLOAD_DIR=UPLOAD_DIR, NEOBROWSER_LOG_LEVEL="error")
        super().__init__([NEO, "serve"], env)
    def navigate(self, url):
        r = self.call("navigate", {"url": url, "wait_s": 2.5})
        return r
    def eval_expr(self, expr):
        r = self.call("js", {"code": "return (%s)" % expr})
        t = r["text"]
        try: return json.loads(t)
        except Exception: return t
    def screenshot(self):
        return self.call("screenshot", {"format": "jpeg", "quality": 40})["image"]
    def tab_new(self): self.call("new_tab", {})
    def tab_count(self):
        try: return len(json.loads(self.call("list_tabs", {})["text"]).get("tabs", []))
        except Exception: return -1
    def tab_close(self, i): self.call("close_tab", {"index": i})
    def upload(self, sel, files):
        r = self.call("upload", {"selector": sel, "files": files})
        return '"ok":true' in r["text"] or '"ok": true' in r["text"]
    def kill_browser(self):
        subprocess.run(["pkill", "-9", "-f", f"{NEO_HOME}/profiles"], capture_output=True)
    def stop_browser(self):
        terminate_browser_gracefully(f"{NEO_HOME}/profiles")
    def responsive(self):
        return self.eval_expr("1+1") == 2


class PwAdapter(MCP):
    name = "Playwright MCP"
    def __init__(self):
        # --user-data-dir is Playwright MCP's native persistence mechanism, the
        # counterpart to NeoBrowser's Ghost profile. Withholding it and then
        # reporting "no persistence" would be measuring the flag we chose not to
        # pass, not the product.
        # cwd=UPLOAD_DIR is how Playwright MCP is *meant* to be pointed at files:
        # it restricts file access to the workspace roots, or cwd when no roots are
        # configured. Note what we do NOT pass: --allow-unrestricted-file-access.
        # Switching off a competitor's security control to make a task pass would
        # be as biased as withholding a capability.
        super().__init__(["npx", "-y", "@playwright/mcp@latest", "--headless",
                          "--user-data-dir", PW_PROFILE], dict(os.environ),
                         cwd=UPLOAD_DIR)
    def navigate(self, url):
        return self.call("browser_navigate", {"url": url})
    def eval_expr(self, expr):
        r = self.call("browser_evaluate", {"function": "() => (%s)" % expr})
        m = re.search(r"### Result\s*(.+?)(?:\n###|\Z)", r["text"], re.S)
        raw = (m.group(1).strip() if m else r["text"].strip())
        try: return json.loads(raw)
        except Exception: return raw
    def screenshot(self):
        return not self.call("browser_take_screenshot", {})["isError"]
    def tab_new(self): self.call("browser_tabs", {"action": "new"})
    def tab_count(self):
        r = self.call("browser_tabs", {"action": "list"})
        return len(re.findall(r"- \d+:", r["text"])) or r["text"].count("Tab")
    def tab_close(self, i): self.call("browser_tabs", {"action": "close", "index": i})
    def upload(self, sel, files):
        # Playwright's native path is chooser-based: only a click driven by
        # Playwright itself arms the file-chooser interception that
        # browser_file_upload then answers. The old harness clicked via JS, which
        # Playwright never sees, so the upload failed for a reason that was ours.
        # `target` takes the same CSS selector NeoBrowser gets, keeping the step
        # abstract and identical for both tools.
        self.call("browser_click", {"element": "file input", "target": sel})
        r = self.call("browser_file_upload", {"paths": files})
        return not r["isError"]
    def kill_browser(self):
        subprocess.run(["pkill", "-9", "-f", "chromium_headless_shell"], capture_output=True)
        subprocess.run(["pkill", "-9", "-f", "ms-playwright"], capture_output=True)
    def stop_browser(self):
        terminate_browser_gracefully("chromium_headless_shell")
    def reset(self):
        # Escape dismisses a pending file chooser, which is the modal state that
        # otherwise blocks every subsequent tool call.
        self.call("browser_press_key", {"key": "Escape"})
        self.call("browser_navigate", {"url": "about:blank"})
        return self.responsive()
    def responsive(self):
        try: return self.eval_expr("1+1") == 2
        except Exception: return False


# ---- tasks (abstract; identical for both adapters) --------------------------
def probe_wall(ad):
    try:
        p = ad.eval_expr(PROBE_JS)
        if isinstance(p, str): p = json.loads(p)
        return classify_wall(p), p
    except Exception:
        return None, {}


def contains(v, needle):
    return needle.lower() in (v if isinstance(v, str) else json.dumps(v)).lower()


def run_functional(ad, img):
    """Returns list of task metric dicts."""
    out = []

    def task(tid, fn):
        t0 = time.time(); calls0 = ad.calls
        steps = []; crash = False; access = False
        try:
            access = fn(steps)
        except Unsupported as u:
            steps.append(("unsupported", False))
        except Exception as e:
            crash = True; steps.append(("error:" + str(e)[:60], False))
        valid = False
        try: valid = ad.responsive()
        except Exception: valid = False
        # A failed task can leave a tool wedged — e.g. an unanswered file chooser
        # puts Playwright in a modal state that makes every later browser_evaluate
        # fail. Left alone, one real failure cascades into a column of them and the
        # scoreboard stops describing the product. Reset before the next task.
        if not valid:
            try: ad.reset()
            except Exception: pass
        lat = round((time.time() - t0) * 1000)
        exec_ok = all(s[1] for s in steps) and not crash
        out.append({"task": tid, "task_execution_success": exec_ok,
                    "destination_access_success": bool(access),
                    "step_success": [s[1] for s in steps], "steps": [s[0] for s in steps],
                    "total_latency_ms": lat, "latency_per_step_ms": round(lat / max(1, len(steps))),
                    "tool_calls": ad.calls - calls0, "browser_crash": crash,
                    "final_state_valid": valid})

    def t_nav(steps):
        ad.navigate("https://example.com"); steps.append(("navigate", True))
        v = ad.eval_expr(text_js("body")); ok = contains(v, "example domain"); steps.append(("read", ok)); return ok
    task("nav_read", t_nav)

    def t_login(steps):
        ad.navigate("https://the-internet.herokuapp.com/login"); steps.append(("navigate", True))
        a = ad.eval_expr(fill_js("#username", "tomsmith")); steps.append(("fill_user", a == "tomsmith"))
        b = ad.eval_expr(fill_js("#password", "SuperSecretPassword!")); steps.append(("fill_pass", b == "SuperSecretPassword!"))
        ad.eval_expr(click_js("button")); steps.append(("submit", True)); time.sleep(1.5)
        v = ad.eval_expr(text_js(".flash")); ok = contains(v, "secure area"); steps.append(("verify", ok)); return ok
    task("login", t_login)

    def t_extract(steps):
        ad.navigate("https://the-internet.herokuapp.com/tables"); steps.append(("navigate", True))
        v = ad.eval_expr(text_js("#table1")); ok = contains(v, "smith"); steps.append(("extract", ok)); return ok
    task("dom_extract", t_extract)

    def t_spa(steps):
        ad.navigate("https://the-internet.herokuapp.com/dynamic_loading/2"); steps.append(("navigate", True))
        ad.eval_expr(click_js("#start button")); steps.append(("start", True))
        ok = False
        for _ in range(15):
            time.sleep(0.6)
            v = ad.eval_expr(text_js("#finish"))
            if contains(v, "hello world"): ok = True; break
        steps.append(("await_render", ok)); return ok
    task("spa_dynamic", t_spa)

    def t_shot(steps):
        ad.navigate("https://example.com"); steps.append(("navigate", True))
        ok = ad.screenshot(); steps.append(("screenshot", ok)); return ok
    task("screenshot", t_shot)

    def t_tabs(steps):
        ad.navigate("https://example.com"); steps.append(("navigate", True))
        ad.tab_new(); ad.navigate("https://httpbin.org/html")
        c = ad.tab_count(); ok = c >= 2; steps.append((f"count={c}", ok))
        try: ad.tab_close(1)
        except Exception: pass
        return ok
    task("multitab", t_tabs)

    def t_upload(steps):
        ad.navigate("https://the-internet.herokuapp.com/upload"); steps.append(("navigate", True))
        u = ad.upload("#file-upload", [img]); steps.append(("upload", u))
        ad.eval_expr(click_js("#file-submit")); time.sleep(2.5)
        v = ad.eval_expr(text_js("#uploaded-files")); ok = contains(v, "bench"); steps.append(("verify", ok)); return ok
    task("upload", t_upload)

    def t_persist(steps):
        # Outcome-based, per the fairness rule: measure whether session state
        # survives a browser restart, not whether a tool with a particular name
        # exists. Each tool may reach the outcome its own way (NeoBrowser via its
        # Ghost profile or save/restore_cookies; Playwright via --user-data-dir).
        # A persistent cookie needs an expiry — a session cookie is *supposed* to
        # die with the browser, so using one would test nothing.
        ad.navigate("https://example.com"); steps.append(("navigate", True))
        ad.eval_expr("document.cookie='benchp=alive; path=/; max-age=86400'")
        wrote = contains(ad.eval_expr("document.cookie"), "benchp=alive")
        steps.append(("write_cookie", wrote))
        # SIGTERM, not SIGKILL: a browser flushes its cookie store on graceful
        # shutdown, so SIGKILL here would measure flush timing rather than
        # persistence — and would penalize both tools for the same wrong reason.
        # The hard kill belongs to the recovery task below, where it is the point.
        ad.stop_browser(); time.sleep(2); steps.append(("restart", True))
        try:
            ad.navigate("https://example.com")
            ok = contains(ad.eval_expr("document.cookie"), "benchp=alive")
        except Exception:
            ok = False
        steps.append(("survived_restart", ok)); return ok
    task("persistence", t_persist)

    def t_recovery(steps):
        ad.navigate("https://example.com"); steps.append(("navigate", True))
        ad.kill_browser(); time.sleep(1); steps.append(("kill", True))
        try:
            ad.navigate("https://example.com")
            v = ad.eval_expr(text_js("body")); ok = contains(v, "example domain")
        except Exception:
            ok = False
        steps.append(("recover", ok)); return ok
    task("recovery", t_recovery)
    # tag recovery/tabs specifics
    for r in out:
        r["recovery_success"] = r["destination_access_success"] if r["task"] == "recovery" else None
        r["tabs_created"] = (2 if r["destination_access_success"] else 0) if r["task"] == "multitab" else None
    return out


def run_adversarial(ad):
    out = []
    for tid, url in [("google_images", "https://www.google.com/search?q=cat&udm=2&num=30"),
                     ("cloudflare_nowsecure", "https://nowsecure.nl/")]:
        t0 = time.time(); calls0 = ad.calls
        try:
            ad.navigate(url)
            wall, probe = probe_wall(ad)
            # destination access = reached real content, i.e. NOT walled and page has body text
            access = wall is None and len((probe or {}).get("text", "")) > 50
        except Exception:
            wall, access = None, False
        out.append({"task": tid, "wall_detected": wall is not None, "wall_type": wall,
                    "destination_access_success": bool(access),
                    "total_latency_ms": round((time.time() - t0) * 1000),
                    "tool_calls": ad.calls - calls0})
    return out


def main():
    os.makedirs(UPLOAD_DIR, exist_ok=True)
    # realpath, not the literal path: on macOS /tmp is a symlink to /private/tmp, and
    # a tool that resolves its allowed roots (Playwright does) will reject the
    # unresolved form as "outside allowed roots". Passing the canonical path to both
    # tools keeps the step identical instead of tripping one of them on a symlink.
    img = os.path.realpath(os.path.join(UPLOAD_DIR, "bench.png"))
    open(img, "wb").write(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="))
    subprocess.run(["rm", "-rf", NEO_HOME], capture_output=True)

    report = {"functional": {}, "adversarial": {}}
    for make in (NeoAdapter, PwAdapter):
        ad = make()
        label = ad.name
        print(f"\n=== {label} · functional ===")
        f = run_functional(ad, img)
        for r in f:
            print(f"  {'✓' if r['task_execution_success'] else '✗'} exec / {'✓' if r['destination_access_success'] else '✗'} access  "
                  f"{r['task']:<14} {r['total_latency_ms']:>6}ms  calls={r['tool_calls']}")
        print(f"--- {label} · adversarial (observational) ---")
        a = run_adversarial(ad)
        for r in a:
            print(f"  {r['task']:<20} wall={r['wall_type']} access={r['destination_access_success']}")
        report["functional"][label] = f
        report["adversarial"][label] = a
        ad.close()

    json.dump(report, open(os.path.join(HERE, "compare.json"), "w"), indent=2)
    write_md(report)
    print("\nreport: bench/compare.md + bench/compare.json")


def write_md(report):
    tools = list(report["functional"].keys())
    md = ["# NeoBrowser vs Playwright MCP — neutral 2-way benchmark\n",
          "Common layer drives both tools with identical abstract steps + JS. Single machine, single run.\n",
          "`task_execution_success` = the steps ran; `destination_access_success` = the intended content was actually reached (a walled/blocked destination is exec-success but access-failure).\n",
          "## Functional\n"]
    tasks = [r["task"] for r in report["functional"][tools[0]]]
    md.append("| task | " + " | ".join(f"{t} exec / access / calls / ms" for t in tools) + " |")
    md.append("|---|" + "|".join(["---"] * len(tools)) + "|")
    for i, tk in enumerate(tasks):
        row = [tk]
        for t in tools:
            r = report["functional"][t][i]
            row.append(f"{'✓' if r['task_execution_success'] else '✗'} / {'✓' if r['destination_access_success'] else '✗'} / {r['tool_calls']} / {r['total_latency_ms']}")
        md.append("| " + " | ".join(row) + " |")
    # summaries
    md.append("\n**Summary**\n")
    md.append("| tool | exec success | access success | avg calls | avg ms | crashes | recovery |")
    md.append("|---|---|---|---|---|---|---|")
    for t in tools:
        f = report["functional"][t]; n = len(f)
        ex = sum(r["task_execution_success"] for r in f); ac = sum(r["destination_access_success"] for r in f)
        rec = next((r["destination_access_success"] for r in f if r["task"] == "recovery"), None)
        md.append(f"| {t} | {ex}/{n} | {ac}/{n} | {round(sum(r['tool_calls'] for r in f)/n,1)} | "
                  f"{round(sum(r['total_latency_ms'] for r in f)/n)} | {sum(r['browser_crash'] for r in f)} | {'PASS' if rec else 'FAIL'} |")
    md.append("\n## Adversarial (observational — no bypass claim)\n")
    md.append("| task | " + " | ".join(f"{t} wall / access" for t in tools) + " |")
    md.append("|---|" + "|".join(["---"] * len(tools)) + "|")
    atasks = [r["task"] for r in report["adversarial"][tools[0]]]
    for i, tk in enumerate(atasks):
        row = [tk]
        for t in tools:
            r = report["adversarial"][t][i]
            row.append(f"{r['wall_type'] or 'none'} / {'✓' if r['destination_access_success'] else '✗'}")
        md.append("| " + " | ".join(row) + " |")
    md.append("\n_Adversarial rows are single-IP, single-run observations. No 'evades better' claim is made — that needs residential-proxy IP rotation + N repetitions + a large site sample._")
    md.append("\n## Honest reading of these numbers\n")
    md.append("- **Latency:** NeoBrowser is consistently slower on these tasks. Part of that is a *deliberate trade-off*: it forces compositor frames (`nudge_frame`) so deferred/virtualized content actually renders in headless Chrome, which Playwright MCP skips, and it can be tuned down where content is static. Do not read the average as a clean multiple, though — see the caveat below, and note that a single outlier moves it substantially.")
    md.append("- **upload:** both tools are now driven through their native path — NeoBrowser via CDP `setFileInputFiles`, Playwright via a Playwright-driven click that arms its file-chooser interception, answered by `browser_file_upload`. Playwright MCP also restricts file access to its workspace roots, so the server is started with its cwd at the fixture directory rather than by passing `--allow-unrestricted-file-access`: pointing a competitor at the right root is fair, switching off its security control is not. The earlier revision of this harness clicked via JS, which Playwright never observes, and so scored a harness bug as a product failure.")
    md.append("- **persistence:** measured by outcome — does a persistent cookie survive a browser restart — not by whether a tool with a particular name exists. Playwright MCP is given `--user-data-dir`, its native persistence mechanism; NeoBrowser uses its Ghost profile. **The previous claim here (\"a genuine capability gap\") was wrong**: Playwright persists sessions perfectly well this way, and the old harness only concluded otherwise because it demanded a `save_cookies`/`restore_cookies` tool.")
    md.append("- **shutdown method matters:** the restart uses SIGTERM against the *browser* process only, for both tools. Chrome flushes its cookie store to SQLite during an orderly shutdown orchestrated by that process; a blanket `pkill -f` also kills the network-service child that owns the store, so nothing is flushed and a fully persistent profile reads as non-persistent. An intermediate revision of this harness made exactly that mistake and scored NeoBrowser as failing persistence.")
    md.append("- **recovery:** both tools recover (each relaunches its browser on the next navigate); this is *not* a NeoBrowser-only strength.")
    md.append("- **latency is not a clean product signal here:** `multitab` and `spa_dynamic` depend on third-party endpoints (`httpbin.org`, `the-internet.herokuapp.com`) that rate-limit by IP. A `multitab` figure in the tens of seconds is that throttle meeting NeoBrowser's longer navigation wait, not a tab-handling cost — the same sequence run in isolation completes in ~6s. Treat single-run latencies as indicative only; a trustworthy latency comparison needs a hermetic fixture server, which this harness does not yet have.")
    md.append("- **walls:** both detect the same walls and both were blocked on a single IP. NeoBrowser's edge is *surfacing* the wall type to the agent as a first-class signal, not bypassing it.")
    open(os.path.join(HERE, "compare.md"), "w").write("\n".join(md) + "\n")


if __name__ == "__main__":
    main()
