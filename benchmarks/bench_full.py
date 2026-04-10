"""
benchmarks/bench_full.py

Full comparative benchmark: V4 vs V3 vs Playwright (browser-mcp).

Coverage:
  - Navigation + page read
  - Screenshot
  - Page info / page analysis
  - Form fill + submit
  - Find and click
  - Extract links / extract table
  - Search (DuckDuckGo)
  - Browse (HTTP fetch)
  - Scroll
  - JavaScript execution
  - Dismiss overlay
  - Paginate
  - Console capture (debug)
  - Metrics (V4 only)
  - Network log (V4 only)

Usage:
    cd /Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/neorender-v2
    python3 benchmarks/bench_full.py [--runs N] [--skip-v3] [--skip-playwright]
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Callable

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

RESULTS_DIR = Path(__file__).parent / "results"

# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------


@dataclass
class Result:
    name: str
    engine: str        # "v4" | "v3" | "playwright"
    ok: bool
    ms: float
    error: str = ""
    extra: dict = field(default_factory=dict)


@dataclass
class Report:
    runs: int
    results: list[Result] = field(default_factory=list)

    def add(self, r: Result) -> None:
        self.results.append(r)

    def summary(self) -> dict[str, dict]:
        out: dict[str, dict] = {}
        tasks = sorted({r.name for r in self.results})
        engines = sorted({r.engine for r in self.results})
        for task in tasks:
            for eng in engines:
                subset = [r for r in self.results if r.name == task and r.engine == eng]
                if not subset:
                    continue
                durations = [r.ms for r in subset if r.ok]
                out[f"{eng}/{task}"] = {
                    "ok_rate": round(sum(1 for r in subset if r.ok) / len(subset), 2),
                    "median_ms": round(statistics.median(durations), 1) if durations else None,
                    "p95_ms": round(sorted(durations)[int(len(durations) * 0.95)], 1) if len(durations) >= 2 else (round(durations[0], 1) if durations else None),
                    "n": len(subset),
                }
        return out


def timed(fn: Callable) -> tuple[bool, float, str]:
    t0 = time.monotonic()
    try:
        result = fn()
        return True, (time.monotonic() - t0) * 1000, ""
    except Exception as exc:
        return False, (time.monotonic() - t0) * 1000, repr(exc)[:120]


# ---------------------------------------------------------------------------
# V4 benchmark
# ---------------------------------------------------------------------------


def run_v4(runs: int, report: Report) -> None:
    from tools.v4.browser import Browser
    import json as _json

    FORM_URL = "https://httpbin.org/forms/post"
    TABLE_URL = "https://www.w3schools.com/html/html_tables.asp"

    print("\n" + "=" * 60)
    print("V4 benchmark")
    print("=" * 60)

    try:
        b = Browser(profile="bench-v4", pool_size=2)
    except Exception as e:
        print(f"  V4 init failed: {e}")
        return

    try:
        for i in range(runs):
            print(f"  [{i+1}/{runs}] ", end="", flush=True)

            # --- navigate ---
            tab = None
            ok, ms, err = timed(lambda: setattr(sys.modules[__name__], "_v4tab", b.open(FORM_URL, wait_s=2.5)))
            tab = getattr(sys.modules[__name__], "_v4tab", None)
            report.add(Result("01_navigate", "v4", ok, ms, err))
            print(f"nav={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            if not tab or not ok:
                print("(tab failed, skipping rest)")
                for name in ["02_screenshot","03_read","04_page_info","05_analyze","06_js",
                             "07_scroll","08_form_fill","09_submit","10_extract_links",
                             "11_extract_table","12_find_and_click","13_search","14_browse",
                             "15_dismiss_overlay","16_paginate","17_debug","18_metrics","19_network"]:
                    report.add(Result(name, "v4", False, 0, "tab_failed"))
                continue

            # --- screenshot ---
            ok, ms, err = timed(lambda: tab.screenshot())
            report.add(Result("02_screenshot", "v4", ok, ms, err))
            print(f"shot={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- read ---
            ok, ms, err = timed(lambda: tab.js("return document.body.innerText.slice(0,200)"))
            report.add(Result("03_read", "v4", ok, ms, err))
            print(f"read={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- page_info ---
            def _page_info():
                return tab.js("""
                    var els = document.querySelectorAll('a,button,input,select,textarea,[role=button]');
                    return JSON.stringify({url: location.href, title: document.title, interactive: els.length});
                """)
            ok, ms, err = timed(_page_info)
            report.add(Result("04_page_info", "v4", ok, ms, err))
            print(f"pi={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- analyze ---
            def _analyze():
                return tab.js("""
                    var forms = Array.from(document.querySelectorAll('form')).map(function(f,fi){
                        var fields = Array.from(f.querySelectorAll('input,select,textarea')).map(function(el){
                            return {tag:el.tagName,name:el.name,type:el.type};
                        });
                        return {index:fi, fields:fields};
                    });
                    return JSON.stringify({forms:forms, url:location.href});
                """)
            ok, ms, err = timed(_analyze)
            report.add(Result("05_analyze", "v4", ok, ms, err))
            print(f"anlz={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- js execution ---
            ok, ms, err = timed(lambda: tab.js("return Array.from({length:1000}, (_,i)=>i*i).reduce((a,b)=>a+b,0)"))
            report.add(Result("06_js", "v4", ok, ms, err))
            print(f"js={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- scroll ---
            ok, ms, err = timed(lambda: tab.js("window.scrollBy(0,500); return window.scrollY"))
            report.add(Result("07_scroll", "v4", ok, ms, err))
            print(f"scrl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- form_fill ---
            def _form_fill():
                fields_map = {"custname": "Bench Test", "custemail": "bench@v4.com", "comments": "v4 benchmark"}
                for label, value in fields_map.items():
                    lq = label
                    val = value
                    tab.js(f"""
                        var inputs = Array.from(document.querySelectorAll('form input,form textarea'));
                        var t = inputs.find(function(el){{ return (el.name||'').indexOf({_json.dumps(lq)}) !== -1; }});
                        if (t) {{
                            var s = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value') ||
                                    Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value');
                            if (s && s.set) s.set.call(t, {_json.dumps(val)});
                            else t.value = {_json.dumps(val)};
                            t.dispatchEvent(new Event('input',{{bubbles:true}}));
                            t.dispatchEvent(new Event('change',{{bubbles:true}}));
                        }}
                        return !!t;
                    """)
            ok, ms, err = timed(_form_fill)
            report.add(Result("08_form_fill", "v4", ok, ms, err))
            print(f"fill={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- submit ---
            def _submit():
                tab.js("var f=document.querySelector('form'); if(f) f.submit();")
                time.sleep(1.5)
                return tab.js("return location.href")
            ok, ms, err = timed(_submit)
            report.add(Result("09_submit", "v4", ok, ms, err))
            print(f"sub={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # Navigate back to form
            tab.navigate(FORM_URL, wait_s=1.5)

            # --- extract links (use httpbin homepage — has <a href> links) ---
            tab.navigate("https://httpbin.org/", wait_s=1.5)
            def _extract():
                res = tab.js("return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,20).map(function(a){return a.href;}))")
                links = _json.loads(res or "[]")
                if not links:
                    raise ValueError("no links found")
            ok, ms, err = timed(_extract)
            report.add(Result("10_extract_links", "v4", ok, ms, err))
            print(f"lnk={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- extract_table ---
            tab.navigate(TABLE_URL, wait_s=2.5)
            def _extract_table():
                res = tab.js("""
                    return (function() {
                        var table = document.querySelector('table');
                        if (!table) return '[]';
                        var headers = Array.from(table.querySelectorAll('th')).map(function(th){return th.textContent.trim();});
                        var rows = Array.from(table.querySelectorAll('tr')).slice(1).map(function(row){
                            var cells = Array.from(row.querySelectorAll('td')).map(function(td){return td.textContent.trim();});
                            var obj = {};
                            cells.forEach(function(c,i){obj[headers[i]||i]=c;});
                            return obj;
                        });
                        return JSON.stringify(rows.slice(0,5));
                    })()
                """)
                data = _json.loads(res or "[]")
                if not data:
                    raise ValueError("no table rows")
            ok, ms, err = timed(_extract_table)
            report.add(Result("11_extract_table", "v4", ok, ms, err))
            print(f"tbl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            tab.navigate(FORM_URL, wait_s=1.5)

            # --- find_and_click ---
            def _find_click():
                res = tab.js("""
                    return (function() {
                        var btn = Array.from(document.querySelectorAll('button,a,[role=button]')).find(function(e){
                            return e.textContent.toLowerCase().indexOf('submit') !== -1;
                        });
                        if (!btn) return false;
                        // don't actually submit, just verify found
                        return true;
                    })()
                """)
                if not res:
                    raise ValueError("submit button not found")
            ok, ms, err = timed(_find_click)
            report.add(Result("12_find_and_click", "v4", ok, ms, err))
            print(f"fnd={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- search (HTTP, no Chrome) ---
            def _search():
                import urllib.request, urllib.parse, re
                q = urllib.parse.quote_plus("python programming")
                req = urllib.request.Request(
                    f"https://html.duckduckgo.com/html/?q={q}",
                    headers={"User-Agent": "Mozilla/5.0"}
                )
                with urllib.request.urlopen(req, timeout=10) as resp:
                    html = resp.read(200_000).decode("utf-8", errors="replace")
                blocks = html.split("result__body")
                titles = []
                for block in blocks[1:4]:
                    m = re.search(r'<a[^>]*class="result__a"[^>]*>(.+?)</a>', block, re.DOTALL)
                    if m:
                        titles.append(re.sub(r"<[^>]+>", "", m.group(1)).strip())
                if not titles:
                    raise ValueError("no results")
            ok, ms, err = timed(_search)
            report.add(Result("13_search", "v4", ok, ms, err))
            print(f"srch={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- browse (HTTP fetch) ---
            def _browse():
                import urllib.request
                req = urllib.request.Request(
                    "https://httpbin.org/get",
                    headers={"User-Agent": "neo-browser/4"}
                )
                with urllib.request.urlopen(req, timeout=10) as resp:
                    data = _json.loads(resp.read())
                if "url" not in data:
                    raise ValueError("unexpected response")
            ok, ms, err = timed(_browse)
            report.add(Result("14_browse", "v4", ok, ms, err))
            print(f"brws={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- dismiss_overlay (detection only, httpbin has no overlay) ---
            def _overlay():
                res = tab.js("""
                    return (function() {
                        var patterns = ['accept','agree','close'];
                        var els = Array.from(document.querySelectorAll('button,a,[role=button]'));
                        for (var i=0;i<els.length;i++){
                            var txt = els[i].textContent.toLowerCase().trim();
                            for (var j=0;j<patterns.length;j++){
                                if (txt.indexOf(patterns[j]) !== -1){
                                    return JSON.stringify({found:true, text:txt});
                                }
                            }
                        }
                        return JSON.stringify({found:false});
                    })()
                """)
                _json.loads(res or '{}')  # just verify it runs
            ok, ms, err = timed(_overlay)
            report.add(Result("15_dismiss_overlay", "v4", ok, ms, err))
            print(f"ovly={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- paginate detection ---
            def _paginate():
                res = tab.js("""
                    return (function() {
                        var patterns = ['next','siguiente','>>','»'];
                        var els = Array.from(document.querySelectorAll('a,button'));
                        for (var i=0;i<els.length;i++){
                            var txt = els[i].textContent.toLowerCase().trim();
                            for (var j=0;j<patterns.length;j++){
                                if (txt === patterns[j]) return JSON.stringify({found:true,text:txt});
                            }
                        }
                        return JSON.stringify({found:false, note:'no next btn on this page'});
                    })()
                """)
                _json.loads(res or '{}')
            ok, ms, err = timed(_paginate)
            report.add(Result("16_paginate", "v4", ok, ms, err))
            print(f"pg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- debug (console interceptor) ---
            def _debug():
                tab.js("""
                    if (!window.__bench_logs) window.__bench_logs = [];
                    window.__orig_log = console.log;
                    console.log = function() {
                        window.__bench_logs.push(Array.from(arguments).join(' '));
                        window.__orig_log.apply(console, arguments);
                    };
                    console.log('bench_test');
                """)
                logs = tab.js("return JSON.stringify(window.__bench_logs || [])")
                data = _json.loads(logs or "[]")
                # restore
                tab.js("if(window.__orig_log){console.log=window.__orig_log;} window.__bench_logs=[];")
                if "bench_test" not in str(data):
                    raise ValueError("log not captured")
            ok, ms, err = timed(_debug)
            report.add(Result("17_debug", "v4", ok, ms, err))
            print(f"dbg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- metrics (V4 only) ---
            def _metrics():
                raw = tab.send("Performance.getMetrics", {})
                metrics = {m["name"]: m["value"] for m in raw.get("metrics", [])}
                if "JSHeapUsedSize" not in metrics:
                    raise ValueError("missing JSHeapUsedSize")
            tab.send("Performance.enable", {})
            ok, ms, err = timed(_metrics)
            report.add(Result("18_metrics", "v4", ok, ms, err))
            print(f"met={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- network log (V4 only — use b.network_log(tab), not tab.network_log()) ---
            def _network():
                tab.enable_network()
                tab.navigate(FORM_URL, wait_s=1.0)
                log = b.network_log(tab)  # b.network_log, not tab.network_log (proxy doesn't expose it)
                if not isinstance(log, list):
                    raise ValueError("expected list")
            ok, ms, err = timed(_network)
            report.add(Result("19_network", "v4", ok, ms, err))
            print(f"net={'✓' if ok else '✗'}({ms:.0f})", end="", flush=True)

            b.close_tab(tab)
            print()

    finally:
        try:
            b.__exit__(None, None, None)
        except Exception:
            pass


# ---------------------------------------------------------------------------
# V3 benchmark (via direct ChromeTab, same Chrome process)
# ---------------------------------------------------------------------------


def run_v3(runs: int, report: Report) -> None:
    import json as _json
    import urllib.request, urllib.parse, re

    FORM_URL = "https://httpbin.org/forms/post"
    TABLE_URL = "https://www.w3schools.com/html/html_tables.asp"

    print("\n" + "=" * 60)
    print("V3 baseline (raw CDP, no pool/cache)")
    print("=" * 60)

    # Find Chrome port from V3 port file
    port_file = os.path.expanduser("~/.neorender/neo-browser-port.txt")
    chrome_port = None
    if os.path.exists(port_file):
        try:
            p = int(open(port_file).read().strip())
            urllib.request.urlopen(f"http://127.0.0.1:{p}/json/version", timeout=2)
            chrome_port = p
        except Exception:
            pass

    if not chrome_port:
        # Try common ports
        for p in [55715, 9222, 9223, 60069]:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{p}/json/version", timeout=1)
                chrome_port = p
                break
            except Exception:
                pass

    if not chrome_port:
        print("  Chrome not found — skipping V3")
        return

    print(f"  Using Chrome on port {chrome_port}")
    from tools.v4.chrome_tab import ChromeTab

    for i in range(runs):
        print(f"  [{i+1}/{runs}] ", end="", flush=True)

        # V3 pattern: open new tab each time (no pool)
        tab = None

        # --- navigate (open + navigate, V3 style) ---
        def _nav():
            t = ChromeTab.open(chrome_port)
            setattr(sys.modules[__name__], "_v3tab", t)
            t.navigate(FORM_URL, wait_s=2.5)
        ok, ms, err = timed(_nav)
        tab = getattr(sys.modules[__name__], "_v3tab", None)
        report.add(Result("01_navigate", "v3", ok, ms, err))
        print(f"nav={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        if not tab or not ok:
            print("(tab failed, skipping)")
            for name in ["02_screenshot","03_read","04_page_info","05_analyze","06_js",
                         "07_scroll","08_form_fill","09_submit","10_extract_links",
                         "11_extract_table","12_find_and_click","13_search","14_browse",
                         "15_dismiss_overlay","16_paginate","17_debug"]:
                report.add(Result(name, "v3", False, 0, "tab_failed"))
            report.add(Result("18_metrics", "v3", False, 0, "not_in_v3"))
            report.add(Result("19_network", "v3", False, 0, "not_in_v3"))
            continue

        # --- screenshot ---
        ok, ms, err = timed(lambda: tab.screenshot())
        report.add(Result("02_screenshot", "v3", ok, ms, err))
        print(f"shot={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- read ---
        ok, ms, err = timed(lambda: tab.js("return document.body.innerText.slice(0,200)"))
        report.add(Result("03_read", "v3", ok, ms, err))
        print(f"read={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- page_info ---
        def _page_info():
            return tab.js("var els=document.querySelectorAll('a,button,input,select,textarea'); return JSON.stringify({url:location.href,interactive:els.length});")
        ok, ms, err = timed(_page_info)
        report.add(Result("04_page_info", "v3", ok, ms, err))
        print(f"pi={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- analyze ---
        def _analyze():
            return tab.js("var forms=document.querySelectorAll('form'); return JSON.stringify({forms:forms.length, url:location.href});")
        ok, ms, err = timed(_analyze)
        report.add(Result("05_analyze", "v3", ok, ms, err))
        print(f"anlz={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- js ---
        ok, ms, err = timed(lambda: tab.js("return Array.from({length:1000},(_,i)=>i*i).reduce((a,b)=>a+b,0)"))
        report.add(Result("06_js", "v3", ok, ms, err))
        print(f"js={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- scroll ---
        ok, ms, err = timed(lambda: tab.js("window.scrollBy(0,500); return window.scrollY"))
        report.add(Result("07_scroll", "v3", ok, ms, err))
        print(f"scrl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- form fill (V3 style: direct value set, no fuzzy matching) ---
        def _form_fill():
            for name, val in [("custname", "Bench V3"), ("custemail", "bench@v3.com")]:
                tab.js(f"""
                    var el = document.querySelector('input[name="{name}"]');
                    if (el) {{ el.value = {_json.dumps(val)}; el.dispatchEvent(new Event('input',{{bubbles:true}})); }}
                """)
        ok, ms, err = timed(_form_fill)
        report.add(Result("08_form_fill", "v3", ok, ms, err))
        print(f"fill={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- submit ---
        def _submit():
            tab.js("var f=document.querySelector('form'); if(f) f.submit();")
            time.sleep(1.5)
            return tab.js("return location.href")
        ok, ms, err = timed(_submit)
        report.add(Result("09_submit", "v3", ok, ms, err))
        print(f"sub={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        tab.navigate("https://httpbin.org/", wait_s=1.5)

        # --- extract links ---
        def _extract():
            res = tab.js("return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,20).map(function(a){return a.href;}))")
            links = _json.loads(res or "[]")
            if not links:
                raise ValueError("no links")
        ok, ms, err = timed(_extract)
        report.add(Result("10_extract_links", "v3", ok, ms, err))
        print(f"lnk={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- extract_table ---
        tab.navigate(TABLE_URL, wait_s=2.5)
        def _extract_table():
            res = tab.js("""
                var table = document.querySelector('table');
                if (!table) return '[]';
                var headers = Array.from(table.querySelectorAll('th')).map(function(th){return th.textContent.trim();});
                var rows = Array.from(table.querySelectorAll('tr')).slice(1,6).map(function(row){
                    var cells = Array.from(row.querySelectorAll('td')).map(function(td){return td.textContent.trim();});
                    var obj = {};
                    cells.forEach(function(c,i){obj[headers[i]||i]=c;});
                    return obj;
                });
                return JSON.stringify(rows);
            """)
            data = _json.loads(res or "[]")
            if not data:
                raise ValueError("no rows")
        ok, ms, err = timed(_extract_table)
        report.add(Result("11_extract_table", "v3", ok, ms, err))
        print(f"tbl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        tab.navigate(FORM_URL, wait_s=1.5)

        # --- find_and_click ---
        def _find_click():
            res = tab.js("""
                var btn = Array.from(document.querySelectorAll('button,a')).find(function(e){return e.textContent.toLowerCase().indexOf('submit')!==-1;});
                return !!btn;
            """)
            if not res:
                raise ValueError("not found")
        ok, ms, err = timed(_find_click)
        report.add(Result("12_find_and_click", "v3", ok, ms, err))
        print(f"fnd={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- search ---
        def _search():
            q = urllib.parse.quote_plus("python programming")
            req = urllib.request.Request(f"https://html.duckduckgo.com/html/?q={q}", headers={"User-Agent": "Mozilla/5.0"})
            with urllib.request.urlopen(req, timeout=10) as resp:
                html = resp.read(200_000).decode("utf-8", errors="replace")
            if "result__a" not in html:
                raise ValueError("no results in HTML")
        ok, ms, err = timed(_search)
        report.add(Result("13_search", "v3", ok, ms, err))
        print(f"srch={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- browse ---
        def _browse():
            req = urllib.request.Request("https://httpbin.org/get", headers={"User-Agent": "neo-browser/3"})
            with urllib.request.urlopen(req, timeout=10) as resp:
                data = _json.loads(resp.read())
            if "url" not in data:
                raise ValueError("bad response")
        ok, ms, err = timed(_browse)
        report.add(Result("14_browse", "v3", ok, ms, err))
        print(f"brws={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- dismiss_overlay (detection) ---
        def _overlay():
            tab.js("""
                var els = Array.from(document.querySelectorAll('button,a'));
                var found = els.find(function(e){return ['accept','close','agree'].some(function(p){return e.textContent.toLowerCase().indexOf(p)!==-1;});});
                return !!found;
            """)
        ok, ms, err = timed(_overlay)
        report.add(Result("15_dismiss_overlay", "v3", ok, ms, err))
        print(f"ovly={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- paginate ---
        def _paginate():
            tab.js("""
                var found = Array.from(document.querySelectorAll('a,button')).find(function(e){return ['next','>>'].indexOf(e.textContent.toLowerCase().trim())!==-1;});
                return !!found;
            """)
        ok, ms, err = timed(_paginate)
        report.add(Result("16_paginate", "v3", ok, ms, err))
        print(f"pg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # --- debug (V3 has no native capture, simulate with JS) ---
        def _debug():
            tab.js("""
                if (!window.__v3_logs) window.__v3_logs = [];
                var orig = console.log;
                console.log = function(){ window.__v3_logs.push(Array.from(arguments).join(' ')); orig.apply(console,arguments); };
                console.log('bench_v3');
            """)
            res = tab.js("return JSON.stringify(window.__v3_logs||[])")
            tab.js("window.__v3_logs=[]; console.log=console.log;")
            if "bench_v3" not in (res or ""):
                raise ValueError("log not captured")
        ok, ms, err = timed(_debug)
        report.add(Result("17_debug", "v3", ok, ms, err))
        print(f"dbg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

        # Metrics + Network: V3 has no native support
        report.add(Result("18_metrics", "v3", False, 0, "not_in_v3"))
        report.add(Result("19_network", "v3", False, 0, "not_in_v3"))
        print("met=N/A net=N/A")

        tab.close()


# ---------------------------------------------------------------------------
# Playwright benchmark (via browser-mcp subprocess)
# ---------------------------------------------------------------------------


def run_playwright(runs: int, report: Report) -> None:
    """
    Playwright baseline via Playwright Python directly.
    Requires: pip install playwright && playwright install chromium
    """
    print("\n" + "=" * 60)
    print("Playwright baseline")
    print("=" * 60)

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("  Playwright not installed — skipping (pip install playwright)")
        return

    FORM_URL = "https://httpbin.org/forms/post"
    TABLE_URL = "https://www.w3schools.com/html/html_tables.asp"

    import json as _json
    import urllib.request, urllib.parse

    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=True)
        context = browser.new_context()

        for i in range(runs):
            print(f"  [{i+1}/{runs}] ", end="", flush=True)

            page = context.new_page()

            # --- navigate ---
            ok, ms, err = timed(lambda: page.goto(FORM_URL, wait_until="domcontentloaded", timeout=10000))
            report.add(Result("01_navigate", "playwright", ok, ms, err))
            print(f"nav={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            if not ok:
                print("(nav failed, skipping)")
                page.close()
                continue

            # --- screenshot ---
            ok, ms, err = timed(lambda: page.screenshot())
            report.add(Result("02_screenshot", "playwright", ok, ms, err))
            print(f"shot={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- read ---
            ok, ms, err = timed(lambda: page.evaluate("document.body.innerText.slice(0,200)"))
            report.add(Result("03_read", "playwright", ok, ms, err))
            print(f"read={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- page_info ---
            ok, ms, err = timed(lambda: page.evaluate("JSON.stringify({url:location.href,title:document.title,interactive:document.querySelectorAll('a,button,input').length})"))
            report.add(Result("04_page_info", "playwright", ok, ms, err))
            print(f"pi={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- analyze ---
            ok, ms, err = timed(lambda: page.evaluate("JSON.stringify({forms:document.querySelectorAll('form').length})"))
            report.add(Result("05_analyze", "playwright", ok, ms, err))
            print(f"anlz={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- js ---
            ok, ms, err = timed(lambda: page.evaluate("Array.from({length:1000},(_,i)=>i*i).reduce((a,b)=>a+b,0)"))
            report.add(Result("06_js", "playwright", ok, ms, err))
            print(f"js={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- scroll ---
            ok, ms, err = timed(lambda: page.evaluate("window.scrollBy(0,500); window.scrollY"))
            report.add(Result("07_scroll", "playwright", ok, ms, err))
            print(f"scrl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- form fill ---
            def _pw_fill():
                page.fill('input[name="custname"]', "PW Bench")
                page.fill('input[name="custemail"]', "pw@bench.com")
                page.fill('textarea[name="comments"]', "playwright bench")
            ok, ms, err = timed(_pw_fill)
            report.add(Result("08_form_fill", "playwright", ok, ms, err))
            print(f"fill={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- submit ---
            def _pw_submit():
                page.evaluate("document.querySelector('form').submit()")
                page.wait_for_load_state("domcontentloaded", timeout=5000)
                return page.url
            ok, ms, err = timed(_pw_submit)
            report.add(Result("09_submit", "playwright", ok, ms, err))
            print(f"sub={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            page.goto("https://httpbin.org/", wait_until="domcontentloaded", timeout=10000)

            # --- extract links ---
            def _pw_links():
                links = page.evaluate("Array.from(document.querySelectorAll('a[href]')).slice(0,20).map(a=>a.href)")
                if not links:
                    raise ValueError("no links")
            ok, ms, err = timed(_pw_links)
            report.add(Result("10_extract_links", "playwright", ok, ms, err))
            print(f"lnk={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- extract_table ---
            page.goto(TABLE_URL, wait_until="domcontentloaded", timeout=10000)
            def _pw_table():
                data = page.evaluate("""
                    () => {
                        const table = document.querySelector('table');
                        if (!table) return [];
                        const headers = [...table.querySelectorAll('th')].map(th=>th.textContent.trim());
                        return [...table.querySelectorAll('tr')].slice(1,6).map(row=>{
                            const cells = [...row.querySelectorAll('td')].map(td=>td.textContent.trim());
                            const obj={};cells.forEach((c,i)=>{obj[headers[i]||i]=c;});return obj;
                        });
                    }
                """)
                if not data:
                    raise ValueError("no table data")
            ok, ms, err = timed(_pw_table)
            report.add(Result("11_extract_table", "playwright", ok, ms, err))
            print(f"tbl={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            page.goto(FORM_URL, wait_until="domcontentloaded", timeout=10000)

            # --- find_and_click ---
            def _pw_find():
                btn = page.get_by_text("Submit order", exact=True)
                if not btn:
                    raise ValueError("not found")
            ok, ms, err = timed(_pw_find)
            report.add(Result("12_find_and_click", "playwright", ok, ms, err))
            print(f"fnd={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- search (HTTP, same as v3/v4) ---
            def _search():
                q = urllib.parse.quote_plus("python programming")
                req = urllib.request.Request(f"https://html.duckduckgo.com/html/?q={q}", headers={"User-Agent": "Mozilla/5.0"})
                with urllib.request.urlopen(req, timeout=10) as resp:
                    html = resp.read(200_000).decode("utf-8", errors="replace")
                if "result__a" not in html:
                    raise ValueError("no results")
            ok, ms, err = timed(_search)
            report.add(Result("13_search", "playwright", ok, ms, err))
            print(f"srch={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- browse (same HTTP fetch) ---
            def _browse():
                req = urllib.request.Request("https://httpbin.org/get", headers={"User-Agent": "playwright/bench"})
                with urllib.request.urlopen(req, timeout=10) as resp:
                    data = _json.loads(resp.read())
                if "url" not in data:
                    raise ValueError("bad")
            ok, ms, err = timed(_browse)
            report.add(Result("14_browse", "playwright", ok, ms, err))
            print(f"brws={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- dismiss_overlay ---
            def _overlay():
                page.evaluate("Array.from(document.querySelectorAll('button')).find(e=>e.textContent.toLowerCase().includes('accept'))")
            ok, ms, err = timed(_overlay)
            report.add(Result("15_dismiss_overlay", "playwright", ok, ms, err))
            print(f"ovly={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- paginate ---
            ok, ms, err = timed(lambda: page.evaluate("document.querySelector('a[rel=next]')"))
            report.add(Result("16_paginate", "playwright", ok, ms, err))
            print(f"pg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # --- debug ---
            def _debug():
                msgs = []
                page.on("console", lambda m: msgs.append(m.text))
                page.evaluate("console.log('pw_bench_test')")
                time.sleep(0.05)
                if "pw_bench_test" not in " ".join(msgs):
                    raise ValueError("not captured")
            ok, ms, err = timed(_debug)
            report.add(Result("17_debug", "playwright", ok, ms, err))
            print(f"dbg={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # metrics via CDP
            def _metrics():
                cdp = context.new_cdp_session(page)
                cdp.send("Performance.enable", {})
                result = cdp.send("Performance.getMetrics", {})
                metrics = {m["name"]: m["value"] for m in result.get("metrics", [])}
                if "JSHeapUsedSize" not in metrics:
                    raise ValueError("missing")
                cdp.detach()
            ok, ms, err = timed(_metrics)
            report.add(Result("18_metrics", "playwright", ok, ms, err))
            print(f"met={'✓' if ok else '✗'}({ms:.0f})", end=" ", flush=True)

            # network log via route interception count
            def _network():
                reqs = []
                page.on("request", lambda r: reqs.append(r.url))
                page.goto(FORM_URL, wait_until="domcontentloaded", timeout=10000)
                if not reqs:
                    raise ValueError("no requests captured")
            ok, ms, err = timed(_network)
            report.add(Result("19_network", "playwright", ok, ms, err))
            print(f"net={'✓' if ok else '✗'}({ms:.0f})")

            page.close()

        context.close()
        browser.close()


# ---------------------------------------------------------------------------
# Print report
# ---------------------------------------------------------------------------

TASK_LABELS = {
    "01_navigate":       "Navigate",
    "02_screenshot":     "Screenshot",
    "03_read":           "Read",
    "04_page_info":      "Page info",
    "05_analyze":        "Analyze",
    "06_js":             "JS exec",
    "07_scroll":         "Scroll",
    "08_form_fill":      "Form fill",
    "09_submit":         "Submit",
    "10_extract_links":  "Extract links",
    "11_extract_table":  "Extract table",
    "12_find_and_click": "Find+click",
    "13_search":         "Search (HTTP)",
    "14_browse":         "Browse (HTTP)",
    "15_dismiss_overlay":"Dismiss overlay",
    "16_paginate":       "Paginate",
    "17_debug":          "Debug/console",
    "18_metrics":        "Metrics (CDP)",
    "19_network":        "Network log",
}


def print_report(report: Report, engines: list[str]) -> None:
    summary = report.summary()
    tasks = sorted(TASK_LABELS.keys())

    # header
    col_w = 10
    header = f"{'Task':<22}"
    for eng in engines:
        header += f" {'ms|ok%':^{col_w}}"
    header += f"  {'Winner'}"
    print("\n" + "=" * (22 + col_w * len(engines) + 12))
    print("BENCHMARK RESULTS")
    print("=" * (22 + col_w * len(engines) + 12))
    eng_header = f"{'Task':<22}" + "".join(f" {e:^{col_w}}" for e in engines) + "  Winner"
    print(eng_header)
    print("-" * (22 + col_w * len(engines) + 12))

    wins = {e: 0 for e in engines}
    for task in tasks:
        label = TASK_LABELS.get(task, task)
        row = f"{label:<22}"
        best_ms = float("inf")
        best_eng = None
        cells: dict[str, str] = {}
        for eng in engines:
            key = f"{eng}/{task}"
            if key not in summary:
                cells[eng] = "  N/A   "
                continue
            s = summary[key]
            if s["median_ms"] is None:
                cells[eng] = f"  -({s['ok_rate']*100:.0f}%)"
                continue
            cells[eng] = f"{s['median_ms']:>5.0f}ms({s['ok_rate']*100:.0f}%)"
            if s["ok_rate"] > 0 and s["median_ms"] < best_ms:
                best_ms = s["median_ms"]
                best_eng = eng
        for eng in engines:
            row += f" {cells.get(eng, '  N/A  '):^{col_w}}"
        winner = best_eng or "-"
        if best_eng:
            wins[best_eng] += 1
        row += f"  {winner}"
        print(row)

    print("-" * (22 + col_w * len(engines) + 12))
    win_row = f"{'WINS':<22}" + "".join(f" {wins.get(e,0):^{col_w}}" for e in engines)
    print(win_row)
    print("=" * (22 + col_w * len(engines) + 12))


def save_report(report: Report) -> Path:
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    ts = time.strftime("%Y%m%d-%H%M%S")
    path = RESULTS_DIR / f"bench-full-{ts}.json"
    summary = report.summary()
    data = {
        "timestamp": ts,
        "runs": report.runs,
        "summary": summary,
        "raw": [asdict(r) for r in report.results],
    }
    path.write_text(json.dumps(data, indent=2))
    return path


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    p = argparse.ArgumentParser(description="Full neo-browser benchmark: v4 vs v3 vs Playwright")
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--skip-v3", action="store_true")
    p.add_argument("--skip-playwright", action="store_true")
    p.add_argument("--only", choices=["v4", "v3", "playwright"], help="Run only one engine")
    args = p.parse_args()

    report = Report(runs=args.runs)
    engines: list[str] = []

    if args.only:
        if args.only == "v4":
            run_v4(args.runs, report)
            engines = ["v4"]
        elif args.only == "v3":
            run_v3(args.runs, report)
            engines = ["v3"]
        elif args.only == "playwright":
            run_playwright(args.runs, report)
            engines = ["playwright"]
    else:
        run_v4(args.runs, report)
        engines.append("v4")
        if not args.skip_v3:
            run_v3(args.runs, report)
            engines.append("v3")
        if not args.skip_playwright:
            run_playwright(args.runs, report)
            engines.append("playwright")

    print_report(report, engines)
    path = save_report(report)
    print(f"\nResults saved → {path}")


if __name__ == "__main__":
    main()
