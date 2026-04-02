#!/usr/bin/env python3
"""NeoBrowser Benchmark Rig — run all tests and compare."""

import sys, os, argparse, time
from pathlib import Path

# Add parent to path
sys.path.insert(0, str(Path(__file__).parent))

from lib import Timer, ResourceMonitor, log_result, save_results, compute_stats, print_comparison, _results
from adapters import NeoBrowserAdapter, PlaywrightAdapter, FetchAdapter

# ── Test URLs ──
STATIC_URLS = [
    'https://example.com',
    'https://httpbin.org/html',
    'https://info.cern.ch',
]
SPA_URLS = [
    'https://github.com/anthropics',
    'https://news.ycombinator.com',
]

REPEATS = 5  # default repetitions per test


def run_test(name, adapter, fn, repeats=REPEATS):
    """Run a test function N times and log results."""
    results = []
    for i in range(repeats):
        try:
            with Timer() as t:
                extra = fn(adapter) or {}
            r = log_result(name, adapter.name, True, t.ms, **extra)
        except Exception as e:
            r = log_result(name, adapter.name, False, 0, error=str(e)[:200])
        results.append(r)
    return results


# ── Test cases ──

def test_cold_start(adapter_class):
    """1) COLD_START — full init + first action."""
    results = []
    for i in range(REPEATS):
        try:
            with Timer() as t_total:
                with Timer() as t_init:
                    adapter = adapter_class().start()
                with Timer() as t_action:
                    result = adapter.browse('https://example.com')
                adapter.stop()
            r = log_result('cold_start', adapter_class.__name__.replace('Adapter','').lower(),
                          True, t_total.ms,
                          init_time_ms=t_init.ms,
                          first_action_ms=t_action.ms,
                          output_len=len(result or ''))
        except Exception as e:
            r = log_result('cold_start', adapter_class.__name__.replace('Adapter','').lower(),
                          False, 0, error=str(e)[:200])
        results.append(r)
    return results


def test_warm_run(adapter):
    """2) WARM_RUN — reuse process, 3 URLs."""
    def fn(a):
        times = []
        for url in STATIC_URLS:
            with Timer() as t:
                a.browse(url)
            times.append(t.ms)
        return {'avg_time': round(sum(times)/len(times), 1), 'urls': len(times)}
    return run_test('warm_run', adapter, fn)


def test_static_page(adapter):
    """3) STATIC_PAGE — fetch + extract."""
    def fn(a):
        result = a.browse('https://example.com')
        return {'output_tokens': len(result or '') // 4}
    return run_test('static_page', adapter, fn)


def test_spa_heavy(adapter):
    """4) SPA_HEAVY — JS-rendered page."""
    def fn(a):
        with Timer() as t_open:
            a.open('https://github.com/anthropics')
        with Timer() as t_read:
            content = a.read()
        return {
            'open_ms': t_open.ms,
            'read_ms': t_read.ms,
            'output_len': len(content or ''),
        }
    return run_test('spa_heavy', adapter, fn)


def test_action_accuracy(adapter):
    """6) ACTION_ACCURACY — clicks and inputs."""
    def fn(a):
        a.open('https://example.com')
        # Try clicking a known element
        click_result = a.click('More information')
        success = 'N/A' not in str(click_result) and 'failed' not in str(click_result).lower()
        return {'click_success': success}
    return run_test('action_accuracy', adapter, fn)


def test_throughput(adapter, concurrency=5):
    """8) THROUGHPUT — concurrent jobs."""
    import threading
    results_list = []
    errors = []

    def job():
        try:
            with Timer() as t:
                adapter.browse('https://example.com')
            results_list.append(t.ms)
        except Exception as e:
            errors.append(str(e))

    with Timer() as t_total:
        threads = [threading.Thread(target=job) for _ in range(concurrency)]
        for th in threads: th.start()
        for th in threads: th.join(timeout=60)

    ops_per_sec = round(len(results_list) / (t_total.ms / 1000), 2) if t_total.ms > 0 else 0
    log_result('throughput', adapter.name, len(errors) == 0, t_total.ms,
              concurrency=concurrency, ops_per_sec=ops_per_sec,
              failures=len(errors), avg_job_ms=round(sum(results_list)/len(results_list), 1) if results_list else 0)


def test_resource_usage(adapter):
    """9) RESOURCE_USAGE — CPU/RAM monitoring."""
    monitor = ResourceMonitor().start()
    time.sleep(1)  # baseline
    for url in STATIC_URLS:
        adapter.browse(url)
    time.sleep(1)  # settle
    monitor.stop()
    stats = monitor.stats()
    log_result('resource_usage', adapter.name, True, 0, **stats)


def test_session_sync(adapter):
    """10) SESSION_SYNC — measure cookie sync time."""
    if adapter.name != 'neobrowser':
        log_result('session_sync', adapter.name, True, 0, note='N/A')
        return
    # Measure by checking ghost profile size
    ghost_dir = Path.home() / '.neorender' / f'ghost-{os.getpid()}'
    if ghost_dir.exists():
        import shutil
        size = sum(f.stat().st_size for f in ghost_dir.rglob('*') if f.is_file())
        log_result('session_sync', adapter.name, True, 0,
                  profile_size_mb=round(size / 1024 / 1024, 2))
    else:
        log_result('session_sync', adapter.name, True, 0, note='no ghost profile')


# ── Main ──

def main():
    parser = argparse.ArgumentParser(description='NeoBrowser Benchmark Rig')
    parser.add_argument('--tools', nargs='+', default=['neobrowser', 'playwright', 'fetch'],
                       help='Tools to benchmark')
    parser.add_argument('--tests', nargs='+', default=['all'],
                       help='Tests to run (cold_start, warm_run, static_page, spa_heavy, action_accuracy, throughput, resource_usage)')
    parser.add_argument('--repeats', type=int, default=5, help='Repetitions per test')
    args = parser.parse_args()

    global REPEATS
    REPEATS = args.repeats

    # Build adapters
    adapter_map = {
        'neobrowser': NeoBrowserAdapter,
        'playwright': PlaywrightAdapter,
        'fetch': FetchAdapter,
    }

    all_tests = ['cold_start', 'warm_run', 'static_page', 'spa_heavy',
                 'action_accuracy', 'throughput', 'resource_usage', 'session_sync']
    tests_to_run = all_tests if 'all' in args.tests else args.tests

    print(f'\n{"="*60}', file=sys.stderr)
    print(f'  NeoBrowser Benchmark Rig', file=sys.stderr)
    print(f'  Tools: {", ".join(args.tools)}', file=sys.stderr)
    print(f'  Tests: {", ".join(tests_to_run)}', file=sys.stderr)
    print(f'  Repeats: {REPEATS}', file=sys.stderr)
    print(f'{"="*60}\n', file=sys.stderr)

    for tool_name in args.tools:
        if tool_name not in adapter_map:
            print(f'Unknown tool: {tool_name}', file=sys.stderr)
            continue

        print(f'\n--- {tool_name.upper()} ---\n', file=sys.stderr)

        # Cold start is special — creates/destroys adapter each time
        if 'cold_start' in tests_to_run:
            test_cold_start(adapter_map[tool_name])

        # Warm tests need a persistent adapter
        adapter = adapter_map[tool_name]().start()
        try:
            if 'warm_run' in tests_to_run:
                test_warm_run(adapter)
            if 'static_page' in tests_to_run:
                test_static_page(adapter)
            if 'spa_heavy' in tests_to_run and tool_name != 'fetch':
                test_spa_heavy(adapter)
            if 'action_accuracy' in tests_to_run and tool_name != 'fetch':
                test_action_accuracy(adapter)
            if 'throughput' in tests_to_run:
                test_throughput(adapter, concurrency=5)
            if 'resource_usage' in tests_to_run:
                test_resource_usage(adapter)
            if 'session_sync' in tests_to_run:
                test_session_sync(adapter)
        finally:
            adapter.stop()

    # Save and print comparison
    save_results()

    # Print comparisons
    test_names = set(r['test_name'] for r in _results)
    for tn in sorted(test_names):
        tools_results = {}
        for tool in args.tools:
            tool_short = tool
            entries = [r for r in _results if r['test_name'] == tn and r['tool'] == tool_short]
            if not entries:
                # Try class name variant
                entries = [r for r in _results if r['test_name'] == tn and tool_short.lower() in r['tool'].lower()]
            if entries:
                tools_results[tool_short] = entries
        if len(tools_results) > 1:
            print_comparison(tn, tools_results)

    print(f'\nDone. {len(_results)} results logged.', file=sys.stderr)


if __name__ == '__main__':
    main()
