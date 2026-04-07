#!/usr/bin/env python3
"""
NeoBrowser V3 — Benchmark Suite

Honest, reproducible benchmark for open-source publication.
Measures wall-clock time, output size, and token estimates.

Design principles:
  - Cold start (Chrome launch) measured separately from warm operations
  - Each test runs independently with clear preconditions
  - Network-dependent tests (search, browse) have inherent variance — noted
  - ChatGPT tests depend on external API — time is mostly server response
  - Token estimate: ~4 chars/token (conservative, accounts for mixed content)
  - No tricks: times include all overhead (serialization, JS injection, etc.)
"""

import sys, time, json, os, statistics

sys.path.insert(0, os.path.dirname(__file__))

def tokens(text):
    """Conservative token estimate: ~4 chars per token."""
    return len(text) // 4 if text else 0

def run(name, fn, args):
    t0 = time.time()
    try:
        result = fn(args)
        ok = True
    except Exception as e:
        result = f'ERROR: {e}'
        ok = False
    elapsed = time.time() - t0
    text = result if isinstance(result, str) else json.dumps(result, ensure_ascii=False)
    return {
        'name': name,
        'time_s': round(elapsed, 3),
        'chars': len(text),
        'tokens': tokens(text),
        'ok': ok and not text.startswith('ERROR'),
        'output': text
    }

def is_error_response(text):
    """Detect error messages from external services (ChatGPT, etc.)."""
    errors = ['Something went wrong', 'error occurred', 'rate limit', 'too many requests']
    return any(e.lower() in text.lower() for e in errors)

def print_row(r, show_preview=False):
    if not r['ok']:
        status = '\033[31mFAIL\033[0m'
    elif is_error_response(r.get('output', '')):
        status = '\033[33mWARN\033[0m'
        r['warn'] = True
    else:
        status = '\033[32mOK\033[0m'
    print(f'  {r["name"]:<28} {r["time_s"]:>6.2f}s {r["chars"]:>6}ch {r["tokens"]:>5}tk  {status}')
    if show_preview or r.get('warn'):
        preview = r["output"][:120].replace('\n', ' ')
        print(f'    → {preview}')

def section(title):
    print(f'\n\033[1m{"─"*60}\033[0m')
    print(f'\033[1m {title}\033[0m')
    print(f'\033[1m{"─"*60}\033[0m')

def main():
    import neo_browser as nb

    all_results = []
    timestamp = time.strftime('%Y-%m-%d %H:%M:%S')

    print(f'\n\033[1m{"═"*60}\033[0m')
    print(f'\033[1m NeoBrowser V3 Benchmark — {timestamp}\033[0m')
    print(f'\033[1m{"═"*60}\033[0m')
    print(f'  Platform: macOS | Chrome headless | CDP protocol')
    print(f'  Note: Times include all overhead. Network-dependent tests vary.')

    # ── 1. HTTP Tools (no Chrome needed) ──
    section('1. HTTP Tools (no Chrome)')
    http_tests = [
        ('search:ddg', nb.tool_search, {'query': 'rust programming language', 'num': 5}),
        ('browse:example.com', nb.tool_browse, {'url': 'https://example.com'}),
        ('browse:hn', nb.tool_browse, {'url': 'https://news.ycombinator.com'}),
    ]
    for name, fn, args in http_tests:
        r = run(name, fn, args)
        all_results.append(r)
        print_row(r)

    # ── 2. Chrome Cold Start ──
    section('2. Chrome Cold Start (includes launch + cookie sync)')
    # First Chrome call — measures full startup
    r = run('open:cold_start', nb.tool_open, {'url': 'https://example.com', 'wait': 2000})
    all_results.append(r)
    print_row(r)
    print(f'    ↳ Chrome launch + cookie sync + navigation + render')

    # ── 3. Chrome Warm Operations ──
    section('3. Chrome Warm (Chrome already running)')
    warm_tests = [
        ('open:example.com', nb.tool_open, {'url': 'https://example.com', 'wait': 2000}),
        ('open:hn', nb.tool_open, {'url': 'https://news.ycombinator.com', 'wait': 3000}),
        ('open:wikipedia', nb.tool_open, {'url': 'https://en.wikipedia.org/wiki/Python_(programming_language)', 'wait': 4000}),
    ]
    for name, fn, args in warm_tests:
        r = run(name, fn, args)
        all_results.append(r)
        print_row(r)

    # ── 4. Read Modes (same page, measures extraction only) ──
    section('4. Read Modes (Wikipedia already loaded)')
    read_tests = [
        ('read:markdown', nb.tool_read, {'type': 'markdown'}),
        ('read:accessibility', nb.tool_read, {'type': 'accessibility'}),
    ]
    for name, fn, args in read_tests:
        r = run(name, fn, args)
        all_results.append(r)
        print_row(r)

    # ── 5. Interaction ──
    section('5. Interaction')
    r = run('open:form', nb.tool_open, {'url': 'https://httpbin.org/forms/post', 'wait': 2000})
    all_results.append(r)
    print_row(r)

    r = run('fill:5_fields', nb.tool_fill, {'fields': json.dumps({
        'Customer name': 'Benchmark User',
        'Telephone': '+34666000111',
        'E-mail address': 'bench@test.dev',
        'Preferred delivery time': '18:00',
        'Delivery instructions': 'Leave at door'
    })})
    all_results.append(r)
    print_row(r)

    r = run('find:text', nb.tool_find, {'text': 'Submit order'})
    all_results.append(r)
    print_row(r)

    r = run('click:submit', nb.tool_click, {'text': 'Submit order'})
    all_results.append(r)
    print_row(r)

    # ── 6. Utility ──
    section('6. Utility')
    # Navigate to a page with content for js/extract tests
    r = run('open:hn_util', nb.tool_open, {'url': 'https://news.ycombinator.com', 'wait': 3000})
    all_results.append(r)
    print_row(r)

    util_tests = [
        ('screenshot', nb.tool_screenshot, {}),
        ('js:eval', nb.tool_js, {'code': 'return document.title'}),
        ('extract:links', nb.tool_extract, {'type': 'links'}),
        ('status', nb.tool_status, {}),
    ]
    for name, fn, args in util_tests:
        r = run(name, fn, args)
        all_results.append(r)
        print_row(r)

    # ── 7. ChatGPT ──
    section('7. ChatGPT (dedicated tab, includes LLM response time)')
    r = run('gpt:send', nb.tool_gpt, {'message': 'Respond with only: OK', 'raw': True})
    all_results.append(r)
    print_row(r, show_preview=True)
    print(f'    ↳ Includes: tab creation + page load + send + LLM response + API fetch')

    r = run('gpt:history', nb.tool_gpt, {'action': 'history', 'count': 3})
    all_results.append(r)
    print_row(r)

    r = run('gpt:read_last', nb.tool_gpt, {'action': 'read_last'})
    all_results.append(r)
    print_row(r)

    # ── Summary ──
    section('Summary')
    ok_count = sum(1 for r in all_results if r['ok'])
    fail_count = len(all_results) - ok_count
    total_time = sum(r['time_s'] for r in all_results)
    total_tokens = sum(r['tokens'] for r in all_results)

    # Exclude GPT send from "controllable" time (it's mostly server wait)
    gpt_time = next((r['time_s'] for r in all_results if r['name'] == 'gpt:send'), 0)
    cold_time = next((r['time_s'] for r in all_results if r['name'] == 'open:cold_start'), 0)

    print(f'  Tests:        {ok_count}/{len(all_results)} passed' + (f' ({fail_count} failed)' if fail_count else ''))
    print(f'  Total time:   {total_time:.1f}s')
    print(f'    ├─ Cold start:   {cold_time:.1f}s (one-time)')
    print(f'    ├─ ChatGPT:      {gpt_time:.1f}s (mostly server response)')
    print(f'    └─ Operations:   {total_time - gpt_time - cold_time:.1f}s ({len(all_results)-2} ops)')
    print(f'  Total tokens: {total_tokens} (~{total_tokens*4} chars)')

    # Per-category breakdown
    cats = {}
    for r in all_results:
        cat = r['name'].split(':')[0]
        if cat not in cats:
            cats[cat] = {'times': [], 'tokens': 0, 'count': 0}
        cats[cat]['times'].append(r['time_s'])
        cats[cat]['tokens'] += r['tokens']
        cats[cat]['count'] += 1

    print(f'\n  {"Category":<12} {"N":>3} {"Avg":>7} {"Min":>7} {"Max":>7} {"Tokens":>7}')
    print(f'  {"-"*12} {"-"*3} {"-"*7} {"-"*7} {"-"*7} {"-"*7}')
    for cat in ['browse', 'open', 'read', 'fill', 'find', 'click', 'extract', 'search', 'gpt', 'screenshot', 'js', 'status']:
        if cat not in cats: continue
        v = cats[cat]
        avg = statistics.mean(v['times'])
        mn = min(v['times'])
        mx = max(v['times'])
        print(f'  {cat:<12} {v["count"]:>3} {avg:>6.2f}s {mn:>6.2f}s {mx:>6.2f}s {v["tokens"]:>7}')

    # browse vs open comparison
    print(f'\n  \033[1mbrowse (HTTP) vs open (Chrome)\033[0m')
    for name in ['browse:example.com', 'open:example.com', 'browse:hn', 'open:hn']:
        r = next((x for x in all_results if x['name'] == name), None)
        if r:
            method = 'HTTP' if 'browse' in name else 'Chrome'
            print(f'    {name:<24} {r["time_s"]:>5.2f}s  {r["tokens"]:>5}tk  ({method})')

    # Save JSON
    out = {
        'timestamp': timestamp,
        'platform': 'macOS',
        'engine': 'Chrome headless + CDP',
        'results': all_results,
        'summary': {
            'total_tests': len(all_results),
            'passed': ok_count,
            'total_time_s': round(total_time, 2),
            'total_tokens': total_tokens,
            'cold_start_s': round(cold_time, 2),
            'gpt_time_s': round(gpt_time, 2),
            'ops_time_s': round(total_time - gpt_time - cold_time, 2),
        }
    }
    out_path = '/tmp/neo-benchmark.json'
    with open(out_path, 'w') as f:
        json.dump(out, f, indent=2, ensure_ascii=False)
    print(f'\n  Raw data → {out_path}')

if __name__ == '__main__':
    main()
