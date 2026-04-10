# PDR: neo-browser v4 — Post-Benchmark Fixes

**Date:** 2026-04-10  
**Version:** 1.0  
**Status:** Draft  
**Scope:** `tools/v4/server.py` — fix 3 confirmed failures from bench_full.py

---

## Context

Full benchmark ran 2026-04-09 comparing v4 vs v3 vs Playwright across 19 operations (3 runs each).  
Result file: `benchmarks/results/bench-full-20260409-193544.json`

V4 won 7/19 categories. Three concrete failures identified:

---

## Finding 1 — `submit` hardcoded sleep (MEDIUM)

**Location:** `tools/v4/server.py` → `elif name == "submit"`  
**Evidence:** submit median = 1506ms (v4+v3) vs 2ms (Playwright).  
**Root cause:** After calling `form.submit()`, the code does `time.sleep(1.5)` unconditionally.  
Playwright uses `wait_for_load_state("domcontentloaded")` which resolves as soon as the browser fires the event — typically <50ms for httpbin.

**Fix spec:**
- After clicking submit button or calling `form.submit()`, poll `document.readyState` every 100ms up to `max_wait_s` (default 5s)
- Return when `readyState === 'complete'` OR URL changes
- Keep 5s hard cap (some forms do slow POST)
- Return `{ok, url, waited_ms}` instead of just `{ok, method}`

**Interface (unchanged):**
```
submit(selector?: string) → {ok: bool, url: str, waited_ms: int}
```

---

## Finding 2 — `network_log` fails in v4 MCP dispatch (HIGH)

**Location:** `tools/v4/server.py` → `elif name == "19_network"` (benchmark) + actual `network_log` tool  
**Evidence:** `19_network` = 0% success rate. Error: tab.enable_network() not available on pool-acquired tab proxy.  
**Root cause:** The `_AcquiredTab` proxy in `tab_pool.py` does not expose `enable_network()`. The dispatch for `network_log` calls `tab.network_log()` which also may not be on the proxy.

**Fix spec:**
- In `dispatch_tool`, call `tab.send("Network.enable", {})` directly before reading the log (idempotent — safe to call multiple times)
- Then call `tab.network_log()` — if missing on proxy, call `tab._tab.network_log()` or implement inline via `tab.send("Network.getResponseBody", ...)`
- Alternatively: expose `enable_network()` and `network_log()` on `_AcquiredTab` proxy

**Interface:**
```
network_log(url_pattern?: str, limit?: int) → [{url, method, status, duration_ms, size_bytes}]
```

---

## Finding 3 — `extract_links` returns 0% on httpbin.org/forms/post (LOW)

**Location:** `tools/v4/server.py` → `elif name == "extract"` + benchmark test  
**Evidence:** `10_extract_links` = 0% for ALL engines on the benchmark test page.  
**Root cause:** httpbin.org/forms/post has no `<a href>` anchor elements — the benchmark chose a bad test URL.  
The `extract` tool itself is correct (verified working on other pages).

**Fix spec (benchmark only):**
- In `bench_full.py`, change extract_links test URL to a page with known links (e.g., `https://httpbin.org/` or `https://example.com`)
- Add assertion: `len(links) > 0`

---

## Finding 4 — `submit` method uses `[role=button]` as first candidate (LOW)

**Location:** `tools/v4/server.py` submit handler, auto-detect branch  
**Evidence:** `[role=button]` matches ALL role=button elements, not specifically submit. Could click wrong element.  
**Fix spec:** Change selector priority: `button[type=submit]` → `input[type=submit]` → `button:last-of-type` → `[role=button][aria-label*=submit]`

---

## Non-goals

- Do NOT change the MCP protocol or tool names
- Do NOT change Playwright benchmark logic (it's a baseline, not production)
- Do NOT add new tools in this PDR

---

## Acceptance criteria

1. `bench_full.py --runs 3` shows `submit` median < 200ms (all engines comparable)
2. `bench_full.py --runs 3` shows `19_network` ✓ for v4
3. `bench_full.py --runs 3` shows `extract_links` ✓ on corrected URL
4. `python3 -c "import ast; ast.parse(...)"` → OK on server.py
