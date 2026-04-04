#!/usr/bin/env python3
"""NeoBrowser Benchmark Rig — run all 12 tests and compare.

Tests
-----
 1. cold_start       — full init + first action
 2. warm_run         — reuse process across N URLs (cache-busted each rep)
 3. static_page      — fetch + extract static HTML
 4. spa_heavy        — JS-rendered page (open + read)
 5. form_flow        — fill form, submit, verify response
 6. action_accuracy  — click known element, verify outcome
 7. multi_tab        — open N tabs in parallel (neobrowser only)
 8. throughput       — concurrent jobs with distinct URLs
 9. resource_usage   — CPU/RAM monitoring over a workload
10. session_sync     — cookie / profile sync measurement
11. long_run (soak)  — run for N minutes under --soak flag
12. end_to_end       — open -> navigate -> extract -> verify workflow

Flags
-----
  --tools             tools to benchmark (default: all available)
  --tests             subset of test names (default: all except long_run)
  --repeats N         reps per test (default: 5)
  --no-cache          clear adapter cache between every repetition
  --soak MINUTES      enable long_run for N minutes (default: 5)
  --concurrency N     threads for throughput test (default: 5)
"""

import sys, os, argparse, time, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from lib import Timer, ResourceMonitor, log_result, save_results, compute_stats, print_comparison, _results
from adapters import NeoBrowserAdapter, NeoBrowserOrigAdapter, PlaywrightAdapter, FetchAdapter

# ── URL pools ──
# Enough distinct URLs that throughput/warm_run reps hit different pages,
# avoiding cache dedup while staying on reliable public endpoints.
STATIC_URLS = [
    'https://example.com',
    'https://httpbin.org/html',
    'https://info.cern.ch',
    'https://httpbin.org/get',
    'https://httpbin.org/status/200',
    'https://httpbin.org/headers',
    'https://httpbin.org/ip',
    'https://httpbin.org/uuid',
]

SPA_URLS = [
    'https://news.ycombinator.com',
    'https://github.com/anthropics',
]

REPEATS = 5


# ── Helpers ──

def _clear_cache_if_needed(adapter, no_cache: bool):
    if no_cache and hasattr(adapter, 'clear_cache'):
        adapter.clear_cache()


def run_test(name, adapter, fn, repeats=None, no_cache=False):
    """Run fn(adapter) N times, clearing cache if --no-cache, log each rep."""
    if repeats is None:
        repeats = REPEATS
    results = []
    for i in range(repeats):
        _clear_cache_if_needed(adapter, no_cache)
        try:
            with Timer() as t:
                extra = fn(adapter) or {}
            r = log_result(name, adapter.name, True, t.ms, rep=i, **extra)
        except Exception as e:
            r = log_result(name, adapter.name, False, 0, rep=i, error=str(e)[:200])
        results.append(r)
    return results


# ── Test 1: COLD_START ──

def test_cold_start(adapter_class, repeats=None, no_cache=False):
    """1) COLD_START — full init + first action, adapter created fresh each rep."""
    if repeats is None:
        repeats = REPEATS
    results = []
    tool_name = adapter_class.name
    for i in range(repeats):
        try:
            with Timer() as t_total:
                with Timer() as t_init:
                    adapter = adapter_class().start()
                _clear_cache_if_needed(adapter, no_cache)
                with Timer() as t_action:
                    result = adapter.browse('https://example.com')
                adapter.stop()
            r = log_result('cold_start', tool_name, True, t_total.ms,
                           rep=i,
                           init_time_ms=t_init.ms,
                           first_action_ms=t_action.ms,
                           output_len=len(result or ''))
        except Exception as e:
            r = log_result('cold_start', tool_name, False, 0, rep=i, error=str(e)[:200])
        results.append(r)
    return results


# ── Test 2: WARM_RUN ──

def test_warm_run(adapter, no_cache=False):
    """2) WARM_RUN — reuse process, fetch REPEATS distinct URLs one per rep.

    Each rep uses a DIFFERENT URL to avoid cache dedup. With --no-cache the
    cache is cleared before the rep anyway, ensuring network round-trips.
    """
    url_pool = STATIC_URLS  # 8 distinct URLs, more than default REPEATS=5

    results = []
    for i in range(REPEATS):
        url = url_pool[i % len(url_pool)]
        _clear_cache_if_needed(adapter, no_cache)
        try:
            with Timer() as t:
                adapter.browse(url)
            r = log_result('warm_run', adapter.name, True, t.ms, rep=i, url=url)
        except Exception as e:
            r = log_result('warm_run', adapter.name, False, 0, rep=i, url=url, error=str(e)[:200])
        results.append(r)
    return results


# ── Test 3: STATIC_PAGE ──

def test_static_page(adapter, no_cache=False):
    """3) STATIC_PAGE — fetch + measure output size.

    Cache is always cleared before this test so results reflect actual fetch
    time, not accumulated cache from warm_run. This ensures neutral comparison.
    """
    if hasattr(adapter, 'clear_cache'):
        adapter.clear_cache()
    def fn(a):
        result = a.browse('https://example.com')
        return {'output_tokens': len(result or '') // 4}
    return run_test('static_page', adapter, fn, no_cache=True)


# ── Test 4: SPA_HEAVY ──

def test_spa_heavy(adapter, no_cache=False):
    """4) SPA_HEAVY — JS-rendered page (open + read)."""
    urls = SPA_URLS
    results = []
    for i in range(REPEATS):
        url = urls[i % len(urls)]
        _clear_cache_if_needed(adapter, no_cache)
        try:
            with Timer() as t:
                with Timer() as t_open:
                    adapter.open(url)
                with Timer() as t_read:
                    content = adapter.read()
            r = log_result('spa_heavy', adapter.name, True, t.ms,
                           rep=i, url=url,
                           open_ms=t_open.ms,
                           read_ms=t_read.ms,
                           output_len=len(content or ''))
        except Exception as e:
            r = log_result('spa_heavy', adapter.name, False, 0, rep=i, url=url, error=str(e)[:200])
        results.append(r)
    return results


# ── Test 5: FORM_FLOW ──

def test_form_flow(adapter, no_cache=False):
    """5) FORM_FLOW — open httpbin form, fill fields, submit, verify response."""
    FORM_URL = 'https://httpbin.org/forms/post'

    def fn(a):
        a.open(FORM_URL)
        # httpbin form has fields: custname, custtel, custemail, size, etc.
        a.fill('input[name="custname"]', 'BenchBot')
        a.fill('input[name="custtel"]', '555-0100')
        a.fill('input[name="custemail"]', 'bench@example.com')
        a.submit()
        # After submit httpbin returns JSON with the posted fields
        content = a.read()
        verified = 'BenchBot' in (content or '') or 'custname' in (content or '')
        return {'form_verified': verified, 'response_len': len(content or '')}

    return run_test('form_flow', adapter, fn, no_cache=no_cache)


# ── Test 6: ACTION_ACCURACY ──

def test_action_accuracy(adapter, no_cache=False):
    """6) ACTION_ACCURACY — click a known element, verify outcome.

    Uses 'Learn more' which is the current link text on example.com.
    Verified 2025-04: example.com page shows <a>Learn more</a> (not 'More information').
    """
    def fn(a):
        a.open('https://example.com')
        click_result = a.click('Learn more')
        success = (
            'N/A' not in str(click_result)
            and 'failed' not in str(click_result).lower()
        )
        return {'click_success': success}
    return run_test('action_accuracy', adapter, fn, no_cache=no_cache)


# ── Test 7: MULTI_TAB ──

def test_multi_tab(adapter, no_cache=False, tab_count=3):
    """7) MULTI_TAB — open N tabs/pages concurrently.

    Only meaningful for neobrowser (which has a browser with multiple contexts).
    Other adapters are logged as N/A.
    NOTE: neobrowser serializes mutations via _browser_lock, so 'concurrency'
    here means scheduling N operations; actual parallelism depends on the lock.
    """
    if adapter.name != 'neobrowser':
        log_result('multi_tab', adapter.name, True, 0, note='N/A — single-page adapter')
        return

    results_ms = []
    errors = []

    def open_tab(url):
        try:
            with Timer() as t:
                adapter.open(url)
            results_ms.append(t.ms)
        except Exception as e:
            errors.append(str(e)[:200])

    urls = STATIC_URLS[:tab_count]
    _clear_cache_if_needed(adapter, no_cache)
    with Timer() as t_total:
        threads = [threading.Thread(target=open_tab, args=(u,)) for u in urls]
        for th in threads:
            th.start()
        for th in threads:
            th.join(timeout=60)

    log_result('multi_tab', adapter.name, len(errors) == 0, t_total.ms,
               tab_count=tab_count,
               successful_tabs=len(results_ms),
               avg_tab_ms=round(sum(results_ms) / len(results_ms), 1) if results_ms else 0,
               failures=len(errors),
               note='neobrowser serializes via _browser_lock — tabs run sequentially')


# ── Test 8: THROUGHPUT ──

def test_throughput(adapter, concurrency=5, no_cache=False):
    """8) THROUGHPUT — concurrent jobs with DISTINCT URLs.

    Cache is always cleared before this test so accumulated cache from prior
    tests (warm_run, spa_heavy) does not give any adapter an unfair advantage.
    NOTE: neobrowser serializes mutating operations via _browser_lock. The ops
    per second measured here reflects lock contention, not true parallelism.
    This is documented in the result so comparisons are fair.
    """
    if hasattr(adapter, 'clear_cache'):
        adapter.clear_cache()
    results_ms = []
    errors = []

    def job(url):
        _clear_cache_if_needed(adapter, no_cache)
        try:
            with Timer() as t:
                adapter.browse(url)
            results_ms.append(t.ms)
        except Exception as e:
            errors.append(str(e)[:200])

    # Use distinct URLs so cache doesn't collapse N concurrent fetches into 1
    urls = [STATIC_URLS[i % len(STATIC_URLS)] for i in range(concurrency)]

    with Timer() as t_total:
        threads = [threading.Thread(target=job, args=(u,)) for u in urls]
        for th in threads:
            th.start()
        for th in threads:
            th.join(timeout=60)

    ops_per_sec = round(len(results_ms) / (t_total.ms / 1000), 2) if t_total.ms > 0 else 0
    serialized = adapter.name == 'neobrowser'
    log_result('throughput', adapter.name, len(errors) == 0, t_total.ms,
               concurrency=concurrency,
               ops_per_sec=ops_per_sec,
               failures=len(errors),
               avg_job_ms=round(sum(results_ms) / len(results_ms), 1) if results_ms else 0,
               serialized_by_lock=serialized)


# ── Test 9: RESOURCE_USAGE ──

def test_resource_usage(adapter, no_cache=False):
    """9) RESOURCE_USAGE — CPU/RAM during a real workload."""
    monitor = ResourceMonitor().start()
    time.sleep(0.5)  # short baseline sample
    for url in STATIC_URLS[:3]:
        _clear_cache_if_needed(adapter, no_cache)
        try:
            adapter.browse(url)
        except Exception:
            pass
    time.sleep(0.5)  # settle
    monitor.stop()
    stats = monitor.stats()
    duration = monitor.duration_ms()
    log_result('resource_usage', adapter.name, True, duration, **stats)


# ── Test 10: SESSION_SYNC ──

def test_session_sync(adapter, no_cache=False):
    """10) SESSION_SYNC — measure actual cookie/profile sync overhead."""
    if adapter.name != 'neobrowser':
        log_result('session_sync', adapter.name, True, 0, note='N/A')
        return

    # Measure time to write + read back a cookie via the JS bridge
    with Timer() as t:
        try:
            adapter.js('document.cookie = "bench_session=1; path=/"')
            cookie_val = adapter.js('document.cookie')
            synced = 'bench_session' in (cookie_val or '')
        except Exception:
            synced = False

    # Also measure ghost profile size if present
    ghost_dir = Path.home() / '.neorender' / f'ghost-{os.getpid()}'
    size_mb = 0.0
    if ghost_dir.exists():
        import shutil
        size_mb = round(
            sum(f.stat().st_size for f in ghost_dir.rglob('*') if f.is_file()) / 1024 / 1024, 2
        )

    log_result('session_sync', adapter.name, synced, t.ms,
               cookie_synced=synced,
               profile_size_mb=size_mb)


# ── Test 11: LONG_RUN / SOAK ──

def test_long_run(adapter, duration_minutes=5, no_cache=False):
    """11) LONG_RUN — soak test: continuous requests for N minutes.

    Enabled only via --soak flag. Measures stability: error rate, latency
    drift, and memory growth over time.
    """
    end_time = time.perf_counter() + duration_minutes * 60
    iteration = 0
    errors = 0
    latencies = []
    url_pool = STATIC_URLS

    monitor = ResourceMonitor().start()
    print(f'  [SOAK] Running for {duration_minutes}m on {adapter.name}...', file=sys.stderr)

    while time.perf_counter() < end_time:
        url = url_pool[iteration % len(url_pool)]
        _clear_cache_if_needed(adapter, no_cache)
        try:
            with Timer() as t:
                adapter.browse(url)
            latencies.append(t.ms)
        except Exception:
            errors += 1
        iteration += 1
        # Brief pause to avoid hammering endpoints
        time.sleep(0.1)

    monitor.stop()
    res_stats = monitor.stats()

    from lib import compute_stats as cs
    fake_results = [{'success': True, 'duration_ms': ms} for ms in latencies]
    lat_stats = cs(fake_results) if fake_results else {}

    log_result('long_run', adapter.name, errors == 0, monitor.duration_ms(),
               iterations=iteration,
               errors=errors,
               error_rate=round(errors / iteration * 100, 2) if iteration else 0,
               p50_ms=lat_stats.get('p50', 0),
               p95_ms=lat_stats.get('p95', 0),
               **res_stats)


# ── Test 12: END_TO_END ──

def test_end_to_end(adapter, no_cache=False):
    """12) END_TO_END — open -> navigate -> extract -> verify full workflow."""
    def fn(a):
        # Step 1: open a known page
        with Timer() as t_open:
            a.open('https://example.com')

        # Step 2: extract links
        with Timer() as t_extract:
            links = a.extract(type='links')

        # Step 3: verify we got content
        link_count = len((links or '').strip().split('\n')) if links and 'N/A' not in links else 0

        # Step 4: navigate to second URL
        with Timer() as t_nav:
            a.open('https://httpbin.org/html')

        # Step 5: read and verify
        with Timer() as t_read:
            content = a.read()

        verified = content and len(content) > 50 and 'N/A' not in content

        return {
            'open_ms': t_open.ms,
            'extract_ms': t_extract.ms,
            'nav_ms': t_nav.ms,
            'read_ms': t_read.ms,
            'link_count': link_count,
            'end_to_end_verified': bool(verified),
        }

    return run_test('end_to_end', adapter, fn, no_cache=no_cache)


# ── Test 13: READ_TYPES ──

# Types to benchmark in order: fast-JS first, then structured, then playwright baseline.
# Each type is logged as a separate result entry (type= in extra) so the custom table
# can show per-type latency and output size side by side.
READ_TYPE_TIERS = {
    'fast_js':    ['text', 'main', 'headings', 'meta', 'links'],
    'structured': ['markdown', 'posts', 'comments', 'table'],
}

def test_read_types(adapter, repeats=3):
    """13) READ_TYPES — latency + output size for each read type.

    Opens HN once, then measures each type in isolation (repeats times).
    Only meaningful for neobrowser; playwright logs its single inner_text baseline.
    Fetch is N/A.
    """
    if adapter.name == 'fetch':
        log_result('read_types', adapter.name, True, 0, type='N/A', note='fetch has no browser')
        return

    # Open HN — real JS-rendered page: links, posts, no semantic headings
    try:
        adapter.open('https://news.ycombinator.com')
    except Exception as e:
        log_result('read_types', adapter.name, False, 0, type='open', error=str(e)[:120])
        return

    if adapter.name == 'playwright':
        for i in range(repeats):
            with Timer() as t:
                content = adapter.read()
            log_result('read_types', adapter.name, True, t.ms,
                       type='inner_text', tier='baseline',
                       chars=len(content or ''), rep=i)
        return

    # neobrowser: all types
    all_types = READ_TYPE_TIERS['fast_js'] + READ_TYPE_TIERS['structured']
    tier_map = {t: 'fast_js' for t in READ_TYPE_TIERS['fast_js']}
    tier_map.update({t: 'structured' for t in READ_TYPE_TIERS['structured']})

    for ct in all_types:
        for i in range(repeats):
            try:
                with Timer() as t:
                    result = adapter.dispatch('read', {'type': ct})
                log_result('read_types', adapter.name, True, t.ms,
                           type=ct, tier=tier_map.get(ct, '?'),
                           chars=len(result or ''), rep=i)
            except Exception as e:
                log_result('read_types', adapter.name, False, 0,
                           type=ct, tier=tier_map.get(ct, '?'), rep=i,
                           error=str(e)[:100])


def print_read_types_table(results_for_test):
    """Custom table: one row per type, columns per tool, showing p50 latency + avg chars."""
    from lib import compute_stats

    print(f'\n{"="*70}')
    print(f'  read_types  (p50 latency ms | avg output chars)')
    print(f'{"="*70}')

    tools = list(results_for_test.keys())
    header = f'  {"type":<14} {"tier":<12}'
    for tool in tools:
        header += f' {tool:>18}'
    print(header)
    print(f'  {"-"*14} {"-"*12}' + ''.join(f' {"-"*18}' for _ in tools))

    # Collect all types across all tools
    all_type_entries = {}  # type -> tier
    for tool, entries in results_for_test.items():
        for e in entries:
            ct = e.get('type', '?')
            tier = e.get('tier', '?')
            all_type_entries[ct] = tier

    # Sort: fast_js first, then structured, then baseline
    tier_order = {'fast_js': 0, 'structured': 1, 'baseline': 2, '?': 3, 'N/A': 4}
    sorted_types = sorted(all_type_entries.keys(),
                          key=lambda t: (tier_order.get(all_type_entries[t], 9), t))

    for ct in sorted_types:
        tier = all_type_entries[ct]
        row = f'  {ct:<14} {tier:<12}'
        for tool in tools:
            entries = [e for e in results_for_test[tool] if e.get('type') == ct]
            if not entries:
                row += f' {"—":>18}'
            elif not any(e.get('success') for e in entries):
                row += f' {"FAIL":>18}'
            else:
                ok = [e for e in entries if e.get('success')]
                p50 = sorted(e['duration_ms'] for e in ok)[len(ok) // 2]
                avg_chars = round(sum(e.get('chars', 0) for e in ok) / len(ok))
                row += f' {p50:>7.1f}ms {avg_chars:>7}ch'
        print(row)
    print()


# ── Main ──

ALL_TESTS = [
    'cold_start', 'warm_run', 'static_page', 'spa_heavy',
    'form_flow', 'action_accuracy', 'multi_tab', 'throughput',
    'resource_usage', 'session_sync', 'end_to_end', 'read_types',
    # 'long_run' excluded from default — use --soak to enable
]

ADAPTER_MAP = {
    'neobrowser': NeoBrowserAdapter,
    'neo-orig': NeoBrowserOrigAdapter,
    'playwright': PlaywrightAdapter,
    'fetch': FetchAdapter,
}

# Tests that don't make sense for fetch-only baseline
BROWSER_ONLY_TESTS = {'spa_heavy', 'action_accuracy', 'multi_tab', 'form_flow', 'session_sync'}


def build_adapter(tool_name):
    """Instantiate and start an adapter, returning None on failure."""
    cls = ADAPTER_MAP.get(tool_name)
    if cls is None:
        print(f'  Unknown tool: {tool_name}', file=sys.stderr)
        return None
    try:
        return cls().start()
    except ImportError as e:
        print(f'  SKIP {tool_name}: {e}', file=sys.stderr)
        return None
    except Exception as e:
        print(f'  SKIP {tool_name} (start failed): {e}', file=sys.stderr)
        return None


def main():
    parser = argparse.ArgumentParser(description='NeoBrowser Benchmark Rig')
    parser.add_argument('--tools', nargs='+', default=list(ADAPTER_MAP.keys()),
                        help='Tools to benchmark (default: all)')
    parser.add_argument('--tests', nargs='+', default=['all'],
                        help='Tests to run (default: all except long_run)')
    parser.add_argument('--repeats', type=int, default=5,
                        help='Repetitions per test (default: 5)')
    parser.add_argument('--no-cache', action='store_true',
                        help='Clear adapter page cache before each test repetition')
    parser.add_argument('--soak', type=float, default=0,
                        metavar='MINUTES',
                        help='Enable long_run soak test for N minutes (default: disabled)')
    parser.add_argument('--concurrency', type=int, default=5,
                        help='Concurrent threads for throughput test (default: 5)')
    args = parser.parse_args()

    global REPEATS
    REPEATS = args.repeats
    no_cache = args.no_cache

    tests_to_run = ALL_TESTS if 'all' in args.tests else args.tests
    if args.soak > 0 and 'long_run' not in tests_to_run:
        tests_to_run = list(tests_to_run) + ['long_run']

    print(f'\n{"="*60}', file=sys.stderr)
    print(f'  NeoBrowser Benchmark Rig', file=sys.stderr)
    print(f'  Tools    : {", ".join(args.tools)}', file=sys.stderr)
    print(f'  Tests    : {", ".join(tests_to_run)}', file=sys.stderr)
    print(f'  Repeats  : {REPEATS}', file=sys.stderr)
    print(f'  No-cache : {no_cache}', file=sys.stderr)
    if args.soak > 0:
        print(f'  Soak     : {args.soak}m', file=sys.stderr)
    print(f'{"="*60}\n', file=sys.stderr)

    for tool_name in args.tools:
        if tool_name not in ADAPTER_MAP:
            print(f'Unknown tool: {tool_name}', file=sys.stderr)
            continue

        print(f'\n--- {tool_name.upper()} ---\n', file=sys.stderr)

        # COLD_START: creates a fresh adapter each rep — handled separately
        if 'cold_start' in tests_to_run:
            test_cold_start(ADAPTER_MAP[tool_name], repeats=REPEATS, no_cache=no_cache)

        # All other tests share a persistent adapter
        adapter = build_adapter(tool_name)
        if adapter is None:
            print(f'  Could not start {tool_name}, skipping warm tests.', file=sys.stderr)
            continue

        # Kick off background Chrome pre-warm now — Chrome will be ready by the time
        # spa_heavy/form_flow/action_accuracy run (after warm_run + static_page).
        # This avoids paying Chrome startup cost on the first browser test.
        if hasattr(adapter, 'prewarm'):
            adapter.prewarm()

        is_browser = tool_name != 'fetch'

        try:
            if 'warm_run' in tests_to_run:
                test_warm_run(adapter, no_cache=no_cache)

            if 'static_page' in tests_to_run:
                test_static_page(adapter, no_cache=no_cache)

            if 'spa_heavy' in tests_to_run and is_browser:
                test_spa_heavy(adapter, no_cache=no_cache)

            if 'form_flow' in tests_to_run and is_browser:
                test_form_flow(adapter, no_cache=no_cache)

            if 'action_accuracy' in tests_to_run and is_browser:
                test_action_accuracy(adapter, no_cache=no_cache)

            if 'multi_tab' in tests_to_run:
                test_multi_tab(adapter, no_cache=no_cache)

            if 'throughput' in tests_to_run:
                test_throughput(adapter, concurrency=args.concurrency, no_cache=no_cache)

            if 'resource_usage' in tests_to_run:
                test_resource_usage(adapter, no_cache=no_cache)

            if 'session_sync' in tests_to_run:
                test_session_sync(adapter, no_cache=no_cache)

            if 'end_to_end' in tests_to_run:
                test_end_to_end(adapter, no_cache=no_cache)

            if 'read_types' in tests_to_run and is_browser:
                test_read_types(adapter, repeats=REPEATS)

            if 'long_run' in tests_to_run and args.soak > 0:
                test_long_run(adapter, duration_minutes=args.soak, no_cache=no_cache)

        finally:
            adapter.stop()

    # ── Save results ──
    save_results()

    # ── Comparison tables — shown for 1+ tools ──
    test_names = sorted(set(r['test_name'] for r in _results))
    for tn in test_names:
        tools_results = {}
        for tool in args.tools:
            entries = [r for r in _results if r['test_name'] == tn and r['tool'] == tool]
            if not entries:
                # fallback: partial name match (e.g. class name variants)
                entries = [r for r in _results if r['test_name'] == tn and tool.lower() in r['tool'].lower()]
            if entries:
                tools_results[tool] = entries
        if not tools_results:
            continue
        # read_types gets a custom table showing per-type latency + chars
        if tn == 'read_types':
            print_read_types_table(tools_results)
        else:
            print_comparison(tn, tools_results)

    print(f'\nDone. {len(_results)} results logged.', file=sys.stderr)


if __name__ == '__main__':
    main()
