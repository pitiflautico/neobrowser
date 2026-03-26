#!/usr/bin/env python3
"""
Ghost Browser — undetectable Chrome with persistent sessions.

Uses undetected-chromedriver to bypass Cloudflare/bot detection.

Usage:
  python3 ghost.py test                                    Bot detection test
  python3 ghost.py open <url> [--profile "Profile 24"]     Open URL with cookies
  python3 ghost.py pong <url> --message "hi" [--profile P] Send chat message
  python3 ghost.py screenshot <url> [--output file.png]    Screenshot
  python3 ghost.py find "text" [--by text|css|xpath|role]  Find element
  python3 ghost.py click "text or selector" [--index 0]    Click element
  python3 ghost.py type "selector" --value "text"          Type into input
  python3 ghost.py scroll [--direction down|up] [--amount 500]  Scroll page
  python3 ghost.py download <url> [--output /tmp/file.pdf]     Download file
  python3 ghost.py monitor <url> --selector "div.price" [--interval 5]  Monitor element
  python3 ghost.py intercept <url> [--pattern "/api/"]         Intercept API calls
  python3 ghost.py cookies <url> [--action list|export|clear]  Manage cookies
  python3 ghost.py tabs --action new|list|switch|close [--url] Tab management
  python3 ghost.py wait <url> --for "selector|text" [--timeout 30]  Wait for element
  python3 ghost.py pipeline --steps '[{...}]' [--stop-on-error] Multi-step pipeline
  python3 ghost.py search "query" [--engine google|ddg] [--num 10]  Web search
  python3 ghost.py read <url> [--selector "main"]                   Extract text
  python3 ghost.py nav <url> [--wait ms]                            Light navigate
  python3 ghost.py fill_form <url> --fields '{"email":"a@b.com"}'  Fill form fields
  python3 ghost.py submit [--selector "form"] [--button "text"]    Submit form
  python3 ghost.py extract <url> --type table|list|product|links   Extract structured data
  python3 ghost.py login <url> --email "u@e.com" --password "p"    Auto-login

Options:
  --session <name>   Session name for persistence (default: "default")
  --profile <name>   Chrome profile to import cookies from
  --headed           Keep browser visible
  --wait <ms>        Wait after load (default: 5000)
"""

import json, os, sys, time, subprocess
from pathlib import Path

args = sys.argv[1:]
command = args[0] if args else 'help'
url = next((a for a in args[1:] if a.startswith('http')), None)

def get_arg(flag):
    try: i = args.index(flag); return args[i + 1] if i + 1 < len(args) else None
    except ValueError: return None

session_name = get_arg('--session') or 'default'
profile_name = get_arg('--profile')
headed = '--headed' in args
wait_ms = int(get_arg('--wait') or '5000')
message = get_arg('--message')
output_file = get_arg('--output')
engine = get_arg('--engine') or 'google'
num_results = int(get_arg('--num') or '10')
selector = get_arg('--selector')
by_strategy = get_arg('--by') or 'text'
value = get_arg('--value')
direction = get_arg('--direction') or 'down'
amount = int(get_arg('--amount') or '500')
index = int(get_arg('--index') or '0')
fields_json = get_arg('--fields')
pattern = get_arg('--pattern') or '/api/'
interval = int(get_arg('--interval') or '5')
action = get_arg('--action') or 'list'
wait_for = get_arg('--for')
steps_json = get_arg('--steps')
timeout = int(get_arg('--timeout') or '30')
stop_on_error = '--stop-on-error' in args
extract_type = get_arg('--type')
button_text = get_arg('--button')
email_arg = get_arg('--email')
password_arg = get_arg('--password')

SESSIONS_DIR = Path.home() / '.neorender' / 'ghost-sessions'
SESSION_DIR = SESSIONS_DIR / session_name
SESSION_DIR.mkdir(parents=True, exist_ok=True)


def import_cookies(domain):
    """Import cookies from Chrome profile."""
    if not profile_name: return []
    script = Path(__file__).parent / 'import-cookies.mjs'
    try:
        r = subprocess.run(['node', str(script), domain, '--profile', profile_name, '--json'],
                           capture_output=True, text=True, timeout=15)
        if r.returncode == 0:
            cookies = json.loads(r.stdout)
            print(f'[ghost] {len(cookies)} cookies for {domain}', file=sys.stderr)
            return cookies
    except Exception as e:
        print(f'[ghost] Cookie import error: {e}', file=sys.stderr)
    return []


def create_driver():
    """Create undetected Chrome driver with persistent profile."""
    import undetected_chromedriver as uc

    options = uc.ChromeOptions()
    options.add_argument('--window-size=1920,1080')
    options.add_argument('--no-sandbox')
    options.add_argument('--disable-dev-shm-usage')
    # NEOMODE: real Chrome UA (remove "HeadlessChrome")
    options.add_argument('--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36')

    if headed:
        pass  # Normal visible window
    else:
        # NEOMODE: headless but indistinguishable from real Chrome
        options.headless = True

    # Retry launch (zombie Chrome processes may block ports)
    driver = None
    for attempt in range(3):
        try:
            driver = uc.Chrome(options=options, version_main=146)
            break
        except Exception as e:
            print(f'[ghost] Launch attempt {attempt+1} failed: {e}', file=sys.stderr)
            os.system("killall -9 'Google Chrome for Testing' 2>/dev/null; killall -9 chromedriver 2>/dev/null")
            time.sleep(3)
    if not driver:
        print('[ghost] Could not start Chrome after 3 attempts', file=sys.stderr)
        sys.exit(1)

    # NEOMODE patches: fix the 5 differences between headless and real Chrome
    if not headed:
        driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': '''
            Object.defineProperty(screen, 'width', {get: () => 1920});
            Object.defineProperty(screen, 'height', {get: () => 1080});
            Object.defineProperty(screen, 'availWidth', {get: () => 1920});
            Object.defineProperty(screen, 'availHeight', {get: () => 1055});
            Object.defineProperty(window, 'outerHeight', {get: () => 1055});
            Object.defineProperty(window, 'innerHeight', {get: () => 968});
        '''})
    print(f'[ghost] Chrome started (session: {session_name})', file=sys.stderr)
    return driver


def inject_cookies(driver, url_str):
    """Import and set cookies from Chrome profile."""
    if not profile_name: return

    from urllib.parse import urlparse
    domain = urlparse(url_str).hostname.replace('www.', '')

    # Get cookies for domain and parent
    all_cookies = import_cookies(domain)
    parts = domain.split('.')
    if len(parts) > 2:
        all_cookies += import_cookies('.'.join(parts[-2:]))

    # Navigate to domain first (required for setting cookies)
    driver.get(url_str)
    time.sleep(1)

    ok = 0
    for c in all_cookies:
        try:
            cookie = {'name': c['name'], 'value': c['value']}
            if c.get('domain'): cookie['domain'] = c['domain']
            if c.get('path'): cookie['path'] = c['path']
            if c.get('secure'): cookie['secure'] = c['secure']
            if c.get('httpOnly'): cookie['httpOnly'] = c['httpOnly']
            if c.get('expires') and c['expires'] > 0:
                cookie['expiry'] = c['expires']
            driver.add_cookie(cookie)
            ok += 1
        except Exception:
            pass

    print(f'[ghost] Injected {ok}/{len(all_cookies)} cookies', file=sys.stderr)

    # Reload with cookies
    driver.get(url_str)
    time.sleep(2)


# ── Commands ──

def cmd_test():
    driver = create_driver()
    try:
        print('[ghost] Testing at bot.sannysoft.com...', file=sys.stderr)
        driver.get('https://bot.sannysoft.com')
        time.sleep(5)
        driver.save_screenshot('/tmp/ghost-bottest.png')
        print('[ghost] Screenshot: /tmp/ghost-bottest.png', file=sys.stderr)

        results = driver.execute_script('''
            const rows = document.querySelectorAll('table tr');
            const r = {};
            rows.forEach(row => {
                const cells = row.querySelectorAll('td');
                if (cells.length >= 2) {
                    r[cells[0].innerText.trim()] = {
                        value: cells[1].innerText.trim(),
                        passed: cells[1].className.includes('passed') || !cells[1].className.includes('failed')
                    };
                }
            });
            return r;
        ''')

        passed = sum(1 for v in results.values() if v.get('passed'))
        print(f'\n{"─"*60}')
        print(f'Bot Detection: {passed}/{len(results)} passed')
        print(f'{"─"*60}')
        for k, v in results.items():
            s = '✓' if v.get('passed') else '✗'
            print(f'  {s} {k}: {v["value"]}')
        print(f'{"─"*60}')
    finally:
        driver.quit()


def cmd_open():
    driver = create_driver()
    try:
        # Navigate first, then inject cookies and reload
        print(f'[ghost] Opening {url}...', file=sys.stderr)
        driver.get(url)
        time.sleep(3)

        if profile_name:
            inject_cookies(driver, url)

        time.sleep(wait_ms / 1000)
        ss = output_file or '/tmp/ghost-screenshot.png'
        driver.save_screenshot(ss)
        print(f'[ghost] Screenshot: {ss}', file=sys.stderr)

        info = driver.execute_script('''return {
            title: document.title, url: location.href,
            text: document.body?.innerText?.trim()?.substring(0, 500),
            html: document.documentElement.outerHTML,
            elements: document.querySelectorAll('*').length,
            forms: document.querySelectorAll('form').length,
        }''')
        # Output HTML separately if --html flag
        # Always export session cookies for V2 reuse
        cookies = driver.get_cookies()
        session_file = str(SESSION_DIR / 'cookies.json')
        v2_cookies = []
        for c in cookies:
            v2 = {'name': c['name'], 'value': c['value'], 'domain': c.get('domain', '')}
            if c.get('path'): v2['path'] = c['path']
            if c.get('secure'): v2['secure'] = True
            if c.get('httpOnly'): v2['http_only'] = True
            if c.get('expiry'): v2['expires'] = int(c['expiry'])
            v2_cookies.append(v2)
        with open(session_file, 'w') as f:
            json.dump(v2_cookies, f, indent=2)
        print(f'[ghost] {len(v2_cookies)} cookies saved to {session_file}', file=sys.stderr)

        if '--html' in args:
            print(info.get('html', ''))
        else:
            del info['html']
            print(json.dumps(info, indent=2))
    finally:
        driver.quit()


def cmd_pong():
    if not message:
        print('Usage: ghost.py pong <url> --message "text"', file=sys.stderr); sys.exit(1)

    driver = create_driver()
    try:
        inject_cookies(driver, url)
        time.sleep(3)
        driver.save_screenshot('/tmp/pong-before.png')

        # Detect platform
        platform = driver.execute_script('''
            if (document.querySelector('#prompt-textarea')) return 'chatgpt';
            if (document.querySelector('[placeholder*="want to know"]')) return 'grok';
            if (document.querySelector('textarea')) return 'generic';
            return 'unknown';
        ''')
        print(f'[pong] Platform: {platform}', file=sys.stderr)

        # Type message
        from selenium.webdriver.common.by import By
        from selenium.webdriver.common.keys import Keys
        from selenium.webdriver.support.ui import WebDriverWait
        from selenium.webdriver.support import expected_conditions as EC

        if platform == 'chatgpt':
            el = WebDriverWait(driver, 10).until(EC.presence_of_element_located((By.ID, 'prompt-textarea')))
            el.click()
            time.sleep(0.3)
            el.send_keys(message)
            time.sleep(0.5)
            try:
                btn = driver.find_element(By.CSS_SELECTOR, '[data-testid="send-button"]')
                btn.click()
            except:
                el.send_keys(Keys.RETURN)

        elif platform == 'grok':
            el = driver.find_element(By.CSS_SELECTOR, '[placeholder*="want to know"]')
            el.send_keys(message)
            el.send_keys(Keys.RETURN)

        else:
            el = driver.find_element(By.CSS_SELECTOR, 'textarea')
            el.send_keys(message)
            el.send_keys(Keys.RETURN)

        print(f'[pong] Sent: {message}', file=sys.stderr)

        # Wait for response
        response = None
        for i in range(60):
            time.sleep(1)
            if platform == 'chatgpt':
                streaming = driver.execute_script('''
                    return !!document.querySelector('[data-testid="stop-button"]') ||
                           !!document.querySelector('button[aria-label*="Stop"]');
                ''')
                if not streaming and i > 5:
                    response = driver.execute_script('''
                        const m = document.querySelectorAll('[data-message-author-role="assistant"]');
                        if (m.length) return m[m.length-1].innerText;
                        const md = document.querySelectorAll('.markdown');
                        if (md.length) return md[md.length-1].innerText;
                        return null;
                    ''')
                    if response: break

            elif platform == 'grok':
                if i > 5:
                    response = driver.execute_script('''
                        const b = document.querySelectorAll('[class*="message"]');
                        return b.length > 1 ? b[b.length-1].innerText : null;
                    ''')
                    if response: break

            if i % 5 == 0:
                print(f'[pong] Waiting... ({i}s)', file=sys.stderr)

        driver.save_screenshot('/tmp/pong-after.png')
        print(f'[pong] Response: {(response or "none")[:100]}', file=sys.stderr)
        print(json.dumps({'platform': platform, 'message': message, 'response': response}, indent=2))
    finally:
        driver.quit()


def cmd_search():
    query = next((a for a in args[1:] if not a.startswith('-')), None)
    if not query:
        print('Usage: ghost.py search "query" [--engine google|ddg] [--num 10]', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    try:
        from urllib.parse import quote_plus
        if engine == 'ddg':
            search_url = f'https://html.duckduckgo.com/html/?q={quote_plus(query)}'
        else:
            search_url = f'https://www.google.com/search?q={quote_plus(query)}'

        print(f'[ghost] Searching {engine}: {query}', file=sys.stderr)
        driver.get(search_url)
        time.sleep(3)

        if engine == 'ddg':
            results = driver.execute_script('''
                const items = document.querySelectorAll('.result');
                const out = [];
                for (const el of items) {
                    const titleEl = el.querySelector('.result__title a, .result__a');
                    const snippetEl = el.querySelector('.result__snippet');
                    if (titleEl) {
                        out.push({
                            title: titleEl.innerText.trim(),
                            url: titleEl.href || '',
                            snippet: snippetEl ? snippetEl.innerText.trim() : ''
                        });
                    }
                }
                return out;
            ''')
        else:
            results = driver.execute_script('''
                const items = document.querySelectorAll('div.g');
                const out = [];
                for (const el of items) {
                    const h3 = el.querySelector('h3');
                    const a = el.querySelector('a');
                    const snippet = el.querySelector('[data-sncf], [style*="-webkit-line-clamp"], .VwiC3b');
                    if (h3 && a) {
                        out.push({
                            title: h3.innerText.trim(),
                            url: a.href || '',
                            snippet: snippet ? snippet.innerText.trim() : ''
                        });
                    }
                }
                return out;
            ''')

        results = (results or [])[:num_results]
        print(f'[ghost] Found {len(results)} results', file=sys.stderr)
        print(json.dumps(results, indent=2, ensure_ascii=False))
    finally:
        driver.quit()


def cmd_read():
    if not url:
        print('Usage: ghost.py read <url> [--selector "main"]', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    try:
        print(f'[ghost] Reading {url}...', file=sys.stderr)
        driver.get(url)
        time.sleep(wait_ms / 1000)

        sel = selector
        if sel:
            js = f'''
                const el = document.querySelector({json.dumps(sel)});
                if (!el) return null;
                return {{
                    title: document.title,
                    text: el.innerText.trim(),
                    word_count: el.innerText.trim().split(/\\s+/).length
                }};
            '''
        else:
            js = '''
                const selectors = ['main', 'article', '[role="main"]', '#content', '.content'];
                let el = null;
                for (const s of selectors) {
                    el = document.querySelector(s);
                    if (el) break;
                }
                if (!el) el = document.body;

                const clone = el.cloneNode(true);
                const remove = ['script', 'style', 'nav', 'footer', 'header', 'aside',
                                'iframe', 'noscript', '[role="navigation"]', '[role="banner"]',
                                '[role="contentinfo"]', '.ad', '.ads', '.advertisement'];
                for (const s of remove) {
                    clone.querySelectorAll(s).forEach(n => n.remove());
                }

                const text = clone.innerText.trim();
                return {
                    title: document.title,
                    text: text,
                    word_count: text.split(/\\s+/).length
                };
            '''

        result = driver.execute_script(js)
        if not result:
            result = {'title': '', 'text': '', 'word_count': 0}
            print('[ghost] No content found for selector', file=sys.stderr)

        print(f'[ghost] Extracted {result.get("word_count", 0)} words', file=sys.stderr)
        print(json.dumps(result, indent=2, ensure_ascii=False))
    finally:
        driver.quit()


def cmd_navigate():
    if not url:
        print('Usage: ghost.py nav <url> [--wait ms]', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    try:
        print(f'[ghost] Navigating to {url}...', file=sys.stderr)
        driver.get(url)
        time.sleep(wait_ms / 1000)

        info = driver.execute_script('''
            return {
                title: document.title,
                url: location.href,
                elements: document.querySelectorAll('*').length,
                forms: document.querySelectorAll('form').length,
                inputs: document.querySelectorAll('input, textarea, select').length,
                links_count: document.querySelectorAll('a[href]').length,
                text_preview: document.body ? document.body.innerText.trim().substring(0, 500) : ''
            };
        ''')
        print(f'[ghost] Page loaded: {info.get("title", "")}', file=sys.stderr)
        print(json.dumps(info, indent=2, ensure_ascii=False))
    finally:
        driver.quit()


def cmd_fill_form():
    if not url:
        print('Usage: ghost.py fill_form <url> --fields \'{"email":"a@b.com"}\'', file=sys.stderr); sys.exit(1)
    if not fields_json:
        print('Usage: ghost.py fill_form <url> --fields \'{"field":"value"}\'', file=sys.stderr); sys.exit(1)

    try:
        fields = json.loads(fields_json)
    except json.JSONDecodeError as e:
        print(f'[ghost] Invalid JSON in --fields: {e}', file=sys.stderr); sys.exit(1)

    driver = create_driver()
    try:
        print(f'[ghost] fill_form: navigating to {url}', file=sys.stderr)
        driver.get(url)
        if profile_name:
            inject_cookies(driver, url)
        time.sleep(wait_ms / 1000)

        result = driver.execute_script('''
            const fields = arguments[0];
            const filled = [];
            const skipped = [];
            const errors = [];

            function setVal(el, val) {
                const nativeSet = Object.getOwnPropertyDescriptor(
                    window.HTMLInputElement.prototype, 'value'
                )?.set || Object.getOwnPropertyDescriptor(
                    window.HTMLTextAreaElement.prototype, 'value'
                )?.set;
                if (nativeSet) nativeSet.call(el, val);
                else el.value = val;
                el.dispatchEvent(new Event('input', {bubbles: true}));
                el.dispatchEvent(new Event('change', {bubbles: true}));
            }

            for (const [key, value] of Object.entries(fields)) {
                try {
                    const k = key.toLowerCase();
                    let input = null;

                    // 1. by name attribute
                    input = document.querySelector(
                        `input[name="${key}"], textarea[name="${key}"], select[name="${key}"]`
                    );
                    // 2. by id
                    if (!input) input = document.getElementById(key);
                    // 3. by placeholder (contains, case-insensitive)
                    if (!input) {
                        for (const el of document.querySelectorAll('input, textarea, select')) {
                            if (el.placeholder && el.placeholder.toLowerCase().includes(k)) {
                                input = el; break;
                            }
                        }
                    }
                    // 4. by label text
                    if (!input) {
                        for (const lbl of document.querySelectorAll('label')) {
                            if (lbl.textContent.toLowerCase().includes(k)) {
                                if (lbl.htmlFor) input = document.getElementById(lbl.htmlFor);
                                if (!input) input = lbl.querySelector('input, textarea, select');
                                if (input) break;
                            }
                        }
                    }
                    // 5. by type attribute (email, password, tel, etc.)
                    if (!input) {
                        input = document.querySelector(`input[type="${k}"]`);
                    }

                    if (input) {
                        setVal(input, value);
                        filled.push(key);
                    } else {
                        skipped.push(key);
                    }
                } catch (e) {
                    errors.push(key + ': ' + e.message);
                }
            }
            return {filled: filled.length, filled_fields: filled, skipped, errors};
        ''', fields)

        print(f'[ghost] Filled {result["filled"]}/{len(fields)} fields', file=sys.stderr)
        driver.save_screenshot('/tmp/ghost-fill-form.png')
        print(json.dumps(result, indent=2))
    finally:
        driver.quit()


def cmd_submit():
    driver = create_driver()
    try:
        if url:
            print(f'[ghost] submit: navigating to {url}', file=sys.stderr)
            driver.get(url)
            if profile_name:
                inject_cookies(driver, url)
            time.sleep(wait_ms / 1000)

        form_selector = selector or 'form'
        before_url = driver.current_url

        result = driver.execute_script('''
            const formSel = arguments[0];
            const btnText = arguments[1];
            let clicked = false;
            let method = '';

            // 1. Find button by text if --button provided
            if (btnText) {
                for (const b of document.querySelectorAll(
                    'button, input[type="submit"], input[type="button"], a'
                )) {
                    const txt = (b.textContent || b.value || '').trim().toLowerCase();
                    if (txt.includes(btnText.toLowerCase())) {
                        b.click(); clicked = true; method = 'button_text'; break;
                    }
                }
            }

            // 2. Submit button inside form
            if (!clicked) {
                const form = document.querySelector(formSel);
                if (form) {
                    const btn = form.querySelector(
                        'button[type="submit"], input[type="submit"], button:not([type])'
                    );
                    if (btn) {
                        btn.click(); clicked = true; method = 'submit_button';
                    } else {
                        form.submit(); clicked = true; method = 'form_submit';
                    }
                }
            }

            // 3. Any submit button on page
            if (!clicked) {
                const btn = document.querySelector('button[type="submit"], input[type="submit"]');
                if (btn) { btn.click(); clicked = true; method = 'global_submit'; }
            }

            return {submitted: clicked, method};
        ''', form_selector, button_text)

        if not result.get('submitted'):
            print('[ghost] No submit button or form found', file=sys.stderr)
            print(json.dumps({'submitted': False, 'error': 'no submit element found'}, indent=2))
            return

        print(f'[ghost] Submitted via {result["method"]}, waiting...', file=sys.stderr)
        time.sleep(3)

        after = driver.execute_script('''return {
            new_url: location.href,
            new_title: document.title,
            response_text_preview: (document.body?.innerText || '').substring(0, 500)
        }''')
        after['submitted'] = True
        after['method'] = result['method']
        after['navigated'] = after['new_url'] != before_url

        driver.save_screenshot('/tmp/ghost-submit.png')
        print(f'[ghost] Submit done. Navigated: {after["navigated"]}', file=sys.stderr)
        print(json.dumps(after, indent=2))
    finally:
        driver.quit()


def cmd_extract_data():
    if not url:
        print('Usage: ghost.py extract <url> --type table|list|product|links', file=sys.stderr); sys.exit(1)
    if not extract_type or extract_type not in ('table', 'list', 'product', 'links'):
        print('Usage: ghost.py extract <url> --type table|list|product|links', file=sys.stderr); sys.exit(1)

    driver = create_driver()
    try:
        print(f'[ghost] extract ({extract_type}): navigating to {url}', file=sys.stderr)
        driver.get(url)
        if profile_name:
            inject_cookies(driver, url)
        time.sleep(wait_ms / 1000)

        if extract_type == 'table':
            data = driver.execute_script('''
                const results = [];
                for (const table of document.querySelectorAll('table')) {
                    const headers = [];
                    const hCells = table.querySelectorAll('thead th, thead td, tr:first-child th');
                    hCells.forEach(h => headers.push(h.innerText.trim()));
                    if (headers.length === 0) {
                        const first = table.querySelector('tr');
                        if (first) first.querySelectorAll('td, th').forEach(c => headers.push(c.innerText.trim()));
                    }
                    const rows = [];
                    const bodyRows = table.querySelectorAll('tbody tr');
                    const dataRows = bodyRows.length ? bodyRows : table.querySelectorAll('tr:not(:first-child)');
                    for (const row of dataRows) {
                        const obj = {};
                        row.querySelectorAll('td, th').forEach((c, i) => {
                            obj[headers[i] || `col_${i}`] = c.innerText.trim();
                        });
                        if (Object.keys(obj).length) rows.push(obj);
                    }
                    results.push({headers, rows, row_count: rows.length});
                }
                return results;
            ''')

        elif extract_type == 'list':
            data = driver.execute_script('''
                const results = [];
                for (const list of document.querySelectorAll('ul, ol')) {
                    const items = [];
                    list.querySelectorAll(':scope > li').forEach(li => items.push(li.innerText.trim()));
                    if (items.length) results.push({tag: list.tagName.toLowerCase(), items, count: items.length});
                }
                return results;
            ''')

        elif extract_type == 'product':
            data = driver.execute_script('''
                const r = {};
                const nameEl = document.querySelector(
                    'h1, [itemprop="name"], .product-title, .product-name, .product_title'
                );
                r.name = nameEl ? nameEl.innerText.trim() : null;

                const priceEl = document.querySelector(
                    '[itemprop="price"], .price, .product-price, .product_price, [class*="price"]'
                );
                r.price = priceEl ? priceEl.innerText.trim() : null;
                const priceAttr = document.querySelector('[itemprop="price"][content]');
                if (priceAttr) r.price_value = priceAttr.getAttribute('content');

                const imgEl = document.querySelector(
                    '[itemprop="image"], .product-image img, .product_image img, #main-image, .gallery img'
                );
                r.image = imgEl ? (imgEl.src || imgEl.getAttribute('data-src')) : null;

                const descEl = document.querySelector(
                    '[itemprop="description"], .product-description, .product_description, #description, [class*="description"]'
                );
                r.description = descEl ? descEl.innerText.trim().substring(0, 1000) : null;

                const currEl = document.querySelector('[itemprop="priceCurrency"]');
                r.currency = currEl ? (currEl.getAttribute('content') || currEl.innerText.trim()) : null;

                const availEl = document.querySelector('[itemprop="availability"]');
                r.availability = availEl ? (availEl.getAttribute('content') || availEl.innerText.trim()) : null;

                return r;
            ''')

        elif extract_type == 'links':
            data = driver.execute_script('''
                const links = [];
                document.querySelectorAll('a[href]').forEach(a => {
                    links.push({
                        text: a.innerText.trim() || a.getAttribute('aria-label') || '',
                        href: a.href
                    });
                });
                return links;
            ''')

        driver.save_screenshot('/tmp/ghost-extract.png')
        count = len(data) if isinstance(data, list) else 1
        print(f'[ghost] Extracted {count} {extract_type} item(s)', file=sys.stderr)
        print(json.dumps({'type': extract_type, 'url': url, 'data': data}, indent=2))
    finally:
        driver.quit()


def cmd_login():
    if not url:
        print('Usage: ghost.py login <url> --email "user@example.com" --password "pass"', file=sys.stderr); sys.exit(1)
    if not email_arg or not password_arg:
        print('Usage: ghost.py login <url> --email "user@example.com" --password "pass"', file=sys.stderr); sys.exit(1)

    driver = create_driver()
    try:
        print(f'[ghost] login: navigating to {url}', file=sys.stderr)
        driver.get(url)
        if profile_name:
            inject_cookies(driver, url)
        time.sleep(wait_ms / 1000)

        fill_result = driver.execute_script('''
            const email = arguments[0];
            const password = arguments[1];
            const result = {email_filled: false, password_filled: false, submit_clicked: false};

            function setVal(el, val) {
                const nativeSet = Object.getOwnPropertyDescriptor(
                    window.HTMLInputElement.prototype, 'value'
                )?.set;
                if (nativeSet) nativeSet.call(el, val);
                else el.value = val;
                el.dispatchEvent(new Event('input', {bubbles: true}));
                el.dispatchEvent(new Event('change', {bubbles: true}));
            }

            // Find email/username field
            let emailEl = document.querySelector(
                'input[type="email"], input[name="email"], input[name="username"], input[name="user"], input[name="login"]'
            );
            if (!emailEl) {
                for (const inp of document.querySelectorAll('input[type="text"], input:not([type])')) {
                    const ph = (inp.placeholder || '').toLowerCase();
                    const nm = (inp.name || '').toLowerCase();
                    const id = (inp.id || '').toLowerCase();
                    if (ph.includes('email') || ph.includes('user') ||
                        nm.includes('email') || nm.includes('user') ||
                        id.includes('email') || id.includes('user')) {
                        emailEl = inp; break;
                    }
                }
            }
            // Fallback: first visible text/email input
            if (!emailEl) {
                for (const c of document.querySelectorAll('input[type="text"], input[type="email"], input:not([type])')) {
                    if (c.offsetParent !== null) { emailEl = c; break; }
                }
            }
            if (emailEl) { setVal(emailEl, email); result.email_filled = true; }

            // Find password field
            let passEl = document.querySelector('input[type="password"]');
            if (!passEl) {
                for (const inp of document.querySelectorAll('input')) {
                    const nm = (inp.name || '').toLowerCase();
                    const ph = (inp.placeholder || '').toLowerCase();
                    if (nm.includes('pass') || ph.includes('pass')) { passEl = inp; break; }
                }
            }
            if (passEl) { setVal(passEl, password); result.password_filled = true; }

            // Find and click submit/login button
            const loginWords = ['log', 'sign', 'submit', 'enter', 'iniciar', 'entrar', 'acceder'];
            let submitBtn = null;
            for (const sel of ['button[type="submit"]', 'input[type="submit"]', 'button:not([type])', 'button']) {
                for (const b of document.querySelectorAll(sel)) {
                    const txt = (b.textContent || b.value || '').toLowerCase();
                    if (loginWords.some(w => txt.includes(w))) { submitBtn = b; break; }
                }
                if (submitBtn) break;
            }
            if (!submitBtn) submitBtn = document.querySelector('button[type="submit"], input[type="submit"]');
            if (submitBtn) { submitBtn.click(); result.submit_clicked = true; }

            return result;
        ''', email_arg, password_arg)

        print(f'[ghost] Login fields: email={fill_result["email_filled"]}, pass={fill_result["password_filled"]}, submit={fill_result["submit_clicked"]}', file=sys.stderr)

        # Wait for redirect
        time.sleep(5)

        after = driver.execute_script('''return {
            title: document.title,
            url: location.href
        }''')
        cookies = driver.get_cookies()
        after['logged_in'] = fill_result['email_filled'] and fill_result['password_filled'] and fill_result['submit_clicked']
        after['cookies_count'] = len(cookies)
        after['fields'] = fill_result

        driver.save_screenshot('/tmp/ghost-login.png')
        print(f'[ghost] Login result: url={after["url"]}, cookies={after["cookies_count"]}', file=sys.stderr)
        print(json.dumps(after, indent=2))
    finally:
        driver.quit()



def cmd_find():
    """Find element(s) on current page by text, CSS, XPath, or ARIA role."""
    query = next((a for a in args[1:] if not a.startswith('-')), None)
    if not query:
        print('Usage: ghost.py find "text or selector" [--by text|css|xpath|role]', file=sys.stderr)
        sys.exit(1)

    from selenium.webdriver.common.by import By

    driver = create_driver()
    try:
        if url:
            print(f'[ghost] Navigating to {url}...', file=sys.stderr)
            driver.get(url)
            time.sleep(3)
        else:
            driver.get('about:blank')

        elements = []
        if by_strategy == 'css':
            elements = driver.find_elements(By.CSS_SELECTOR, query)
        elif by_strategy == 'xpath':
            elements = driver.find_elements(By.XPATH, query)
        elif by_strategy == 'role':
            elements = driver.find_elements(By.CSS_SELECTOR, f'[role="{query}"]')
        else:
            # text: try multiple strategies
            elements = driver.find_elements(By.XPATH, f'//*[normalize-space(text())="{query}"]')
            if not elements:
                elements = driver.find_elements(By.XPATH, f'//*[contains(text(), "{query}")]')
            if not elements:
                elements = driver.find_elements(By.CSS_SELECTOR, f'[aria-label*="{query}"]')
            if not elements:
                elements = driver.find_elements(By.CSS_SELECTOR, f'[placeholder*="{query}"]')
            if not elements:
                elements = driver.find_elements(By.CSS_SELECTOR, f'[value*="{query}"]')

        print(f'[ghost] Found {len(elements)} element(s)', file=sys.stderr)

        results = []
        for el in elements[:5]:
            rect = el.rect
            results.append({
                'found': True,
                'tag': el.tag_name,
                'text': (el.text or '')[:200],
                'selector': el.get_attribute('class') or el.get_attribute('id') or '',
                'rect': {'x': rect['x'], 'y': rect['y'], 'w': rect['width'], 'h': rect['height']},
                'clickable': el.is_enabled() and el.is_displayed(),
                'type': el.get_attribute('type') or '',
            })

        if not results:
            print(json.dumps({'found': False, 'query': query, 'strategy': by_strategy}))
        elif len(results) == 1:
            print(json.dumps(results[0], indent=2))
        else:
            print(json.dumps(results, indent=2))
    finally:
        driver.quit()


def cmd_click():
    """Find element by text or CSS selector and click it."""
    query = next((a for a in args[1:] if not a.startswith('-')), None)
    if not query:
        print('Usage: ghost.py click "text or selector" [--index 0]', file=sys.stderr)
        sys.exit(1)

    from selenium.webdriver.common.by import By

    driver = create_driver()
    try:
        if url:
            print(f'[ghost] Navigating to {url}...', file=sys.stderr)
            driver.get(url)
            time.sleep(3)
        else:
            driver.get('about:blank')

        elements = []
        # Try text match first
        elements = driver.find_elements(By.XPATH, f'//*[normalize-space(text())="{query}"]')
        if not elements:
            elements = driver.find_elements(By.XPATH, f'//*[contains(text(), "{query}")]')
        # Fall back to CSS selector
        if not elements:
            try:
                elements = driver.find_elements(By.CSS_SELECTOR, query)
            except Exception:
                pass
        # Try aria-label
        if not elements:
            elements = driver.find_elements(By.CSS_SELECTOR, f'[aria-label*="{query}"]')

        if not elements:
            print(json.dumps({'clicked': False, 'error': f'Element not found: {query}'}))
            return

        idx = min(index, len(elements) - 1)
        el = elements[idx]
        tag = el.tag_name
        text = (el.text or '')[:100]

        print(f'[ghost] Clicking <{tag}> "{text}" (index {idx})', file=sys.stderr)
        el.click()
        time.sleep(2)

        result = {
            'clicked': True,
            'element': {'tag': tag, 'text': text},
            'new_url': driver.current_url,
            'new_title': driver.title,
        }
        print(json.dumps(result, indent=2))
    finally:
        driver.quit()


def cmd_type():
    """Find input by label/placeholder/name/CSS and type into it."""
    query = next((a for a in args[1:] if not a.startswith('-')), None)
    if not query or not value:
        print('Usage: ghost.py type "selector or label" --value "text"', file=sys.stderr)
        sys.exit(1)

    from selenium.webdriver.common.by import By

    driver = create_driver()
    try:
        if url:
            print(f'[ghost] Navigating to {url}...', file=sys.stderr)
            driver.get(url)
            time.sleep(3)
        else:
            driver.get('about:blank')

        el = None
        # 1. Label text -> associated input via for attribute
        labels = driver.find_elements(By.XPATH, f'//label[contains(text(), "{query}")]')
        if labels:
            for_id = labels[0].get_attribute('for')
            if for_id:
                try:
                    el = driver.find_element(By.ID, for_id)
                except Exception:
                    pass
            if not el:
                try:
                    el = labels[0].find_element(By.CSS_SELECTOR, 'input, textarea, select')
                except Exception:
                    pass

        # 2. Placeholder
        if not el:
            found = driver.find_elements(By.CSS_SELECTOR, f'[placeholder*="{query}"]')
            if found:
                el = found[0]

        # 3. Name attribute
        if not el:
            found = driver.find_elements(By.CSS_SELECTOR, f'[name="{query}"]')
            if found:
                el = found[0]

        # 4. CSS selector
        if not el:
            try:
                found = driver.find_elements(By.CSS_SELECTOR, query)
                if found:
                    el = found[0]
            except Exception:
                pass

        if not el:
            print(json.dumps({'typed': False, 'error': f'Field not found: {query}'}))
            return

        tag = el.tag_name
        name = el.get_attribute('name') or ''
        placeholder = el.get_attribute('placeholder') or ''

        print(f'[ghost] Typing into <{tag}> name="{name}"', file=sys.stderr)
        el.clear()
        el.send_keys(value)

        result = {
            'typed': True,
            'field': {'tag': tag, 'name': name, 'placeholder': placeholder},
            'value': value,
        }
        print(json.dumps(result, indent=2))
    finally:
        driver.quit()


def cmd_scroll():
    """Scroll the page up or down."""
    driver = create_driver()
    try:
        if url:
            print(f'[ghost] Navigating to {url}...', file=sys.stderr)
            driver.get(url)
            time.sleep(3)
        else:
            driver.get('about:blank')

        px = amount if direction == 'down' else -amount
        print(f'[ghost] Scrolling {direction} {amount}px', file=sys.stderr)

        driver.execute_script(f'window.scrollBy(0, {px})')
        time.sleep(0.5)

        info = driver.execute_script('''return {
            scroll_y: Math.round(window.scrollY),
            page_height: document.documentElement.scrollHeight,
            viewport_height: window.innerHeight,
            at_bottom: (window.innerHeight + Math.round(window.scrollY)) >= document.documentElement.scrollHeight
        }''')

        result = {
            'scrolled': True,
            'direction': direction,
            'scroll_y': info['scroll_y'],
            'page_height': info['page_height'],
            'at_bottom': info['at_bottom'],
        }
        print(json.dumps(result, indent=2))
    finally:
        driver.quit()


# ── Main ──
if command == 'test':
    cmd_test()
elif command == 'open' and url:
    cmd_open()
elif command == 'pong' and url:
    cmd_pong()
elif command == 'search':
    cmd_search()
elif command == 'read' and url:
    cmd_read()
elif command in ('nav', 'navigate'):
    cmd_navigate()
elif command == 'fill_form':
    cmd_fill_form()
elif command == 'submit':
    cmd_submit()
elif command == 'extract':
    cmd_extract_data()
elif command == 'login':
    cmd_login()
elif command == 'find':
    cmd_find()
elif command == 'click':
    cmd_click()
elif command == 'type':
    cmd_type()
elif command == 'scroll':
    cmd_scroll()
elif command == 'download' and url:
    cmd_download()
elif command == 'monitor' and url:
    cmd_monitor()
elif command == 'intercept' and url:
    cmd_api_intercept()
elif command == 'cookies' and url:
    cmd_cookies()
elif command == 'tabs':
    cmd_tabs()
elif command == 'wait' and url:
    cmd_wait()
elif command == 'pipeline':
    cmd_pipeline()
else:
    print(__doc__)


# ── New Commands (download, monitor, intercept, cookies, tabs, wait, pipeline) ──

def cmd_download():
    """Download a file from URL."""
    import requests as req
    from urllib.parse import urlparse

    out_path = output_file or f'/tmp/{Path(urlparse(url).path).name or "download"}'

    if selector:
        driver = create_driver()
        try:
            from selenium.webdriver.common.by import By
            from selenium.webdriver.support.ui import WebDriverWait
            from selenium.webdriver.support import expected_conditions as EC

            print(f'[ghost] Navigating to {url} for download click...', file=sys.stderr)
            driver.get(url)
            time.sleep(3)

            el = WebDriverWait(driver, 10).until(
                EC.element_to_be_clickable((By.CSS_SELECTOR, selector))
            )
            el.click()
            print(f'[ghost] Clicked download button: {selector}', file=sys.stderr)
            time.sleep(5)

            print(json.dumps({'downloaded': True, 'path': out_path, 'size_bytes': -1, 'method': 'click'}))
        finally:
            driver.quit()
        return

    print(f'[ghost] Downloading {url}...', file=sys.stderr)
    r = req.get(url, stream=True, headers={
        'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
    })
    r.raise_for_status()
    with open(out_path, 'wb') as f:
        for chunk in r.iter_content(8192):
            f.write(chunk)
    size = os.path.getsize(out_path)
    print(f'[ghost] Saved to {out_path} ({size} bytes)', file=sys.stderr)
    print(json.dumps({'downloaded': True, 'path': out_path, 'size_bytes': size}))


def cmd_monitor():
    """Monitor an element for changes."""
    if not selector:
        print('Usage: ghost.py monitor <url> --selector "div.price" [--interval 5]', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    try:
        from selenium.webdriver.common.by import By

        print(f'[ghost] Monitoring {selector} at {url}...', file=sys.stderr)
        driver.get(url)
        time.sleep(3)

        el = driver.find_element(By.CSS_SELECTOR, selector)
        old_value = el.text
        print(f'[ghost] Initial value: {old_value}', file=sys.stderr)

        time.sleep(interval)
        driver.refresh()
        time.sleep(3)

        el = driver.find_element(By.CSS_SELECTOR, selector)
        new_value = el.text
        changed = old_value != new_value

        print(f'[ghost] New value: {new_value} (changed={changed})', file=sys.stderr)
        print(json.dumps({'changed': changed, 'old_value': old_value, 'new_value': new_value, 'selector': selector}))
    finally:
        driver.quit()


def cmd_api_intercept():
    """Intercept XHR/fetch requests via CDP Network domain."""
    driver = create_driver()
    try:
        driver.execute_cdp_cmd('Network.enable', {})
        print(f'[ghost] Network logging enabled, navigating to {url}...', file=sys.stderr)

        driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': '''
            window.__ghost_requests = [];
            const origFetch = window.fetch;
            window.fetch = async function(...args) {
                const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';
                const method = args[1]?.method || 'GET';
                try {
                    const resp = await origFetch.apply(this, args);
                    const clone = resp.clone();
                    let body = '';
                    try { body = await clone.text(); } catch(e) {}
                    window.__ghost_requests.push({
                        url, method, status: resp.status,
                        response_body_preview: body.substring(0, 500)
                    });
                    return resp;
                } catch(e) {
                    window.__ghost_requests.push({url, method, status: 0, response_body_preview: e.message});
                    throw e;
                }
            };
            const origXHR = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__ghost_method = method;
                this.__ghost_url = url;
                this.addEventListener('load', function() {
                    window.__ghost_requests.push({
                        url: this.__ghost_url, method: this.__ghost_method,
                        status: this.status,
                        response_body_preview: (this.responseText || '').substring(0, 500)
                    });
                });
                return origXHR.apply(this, arguments);
            };
        '''})

        driver.get(url)
        time.sleep(5)

        all_reqs = driver.execute_script('return window.__ghost_requests || [];')
        captured = [r for r in all_reqs if pattern in r.get('url', '')]

        print(f'[ghost] Captured {len(captured)} requests matching "{pattern}"', file=sys.stderr)
        print(json.dumps(captured, indent=2))
    finally:
        driver.quit()


def cmd_cookies():
    """Manage cookies for a URL."""
    driver = create_driver()
    try:
        print(f'[ghost] Navigating to {url}...', file=sys.stderr)
        driver.get(url)
        time.sleep(3)

        if profile_name:
            inject_cookies(driver, url)

        if action == 'clear':
            driver.delete_all_cookies()
            print('[ghost] All cookies cleared', file=sys.stderr)
            print(json.dumps({'action': 'clear', 'cookies': []}))

        elif action == 'export':
            cookies = driver.get_cookies()
            export_path = output_file or '/tmp/ghost-cookies.json'
            export_data = [{'name': c['name'], 'value': c['value'], 'domain': c.get('domain', '')} for c in cookies]
            with open(export_path, 'w') as f:
                json.dump(export_data, f, indent=2)
            print(f'[ghost] {len(export_data)} cookies exported to {export_path}', file=sys.stderr)
            print(json.dumps({'action': 'export', 'path': export_path, 'cookies': export_data}))

        else:  # list
            cookies = driver.get_cookies()
            cookie_list = [{'name': c['name'], 'value': c['value'], 'domain': c.get('domain', '')} for c in cookies]
            print(f'[ghost] {len(cookie_list)} cookies found', file=sys.stderr)
            print(json.dumps({'action': 'list', 'cookies': cookie_list}, indent=2))
    finally:
        driver.quit()


def cmd_tabs():
    """Manage browser tabs."""
    driver = create_driver()
    try:
        if action == 'new':
            target_url = url or 'about:blank'
            driver.execute_script(f'window.open("{target_url}");')
            driver.switch_to.window(driver.window_handles[-1])
            time.sleep(2)
            tabs = []
            for i, handle in enumerate(driver.window_handles):
                driver.switch_to.window(handle)
                tabs.append({'title': driver.title, 'url': driver.current_url, 'active': i == len(driver.window_handles) - 1})
            print(json.dumps({'action': 'new', 'tabs': tabs}, indent=2))

        elif action == 'list':
            if url:
                driver.get(url)
                time.sleep(3)
            tabs = []
            current = driver.current_window_handle
            for i, handle in enumerate(driver.window_handles):
                driver.switch_to.window(handle)
                tabs.append({'title': driver.title, 'url': driver.current_url, 'active': handle == current})
            driver.switch_to.window(current)
            print(json.dumps({'action': 'list', 'tabs': tabs}, indent=2))

        elif action == 'switch':
            handles = driver.window_handles
            if index < len(handles):
                driver.switch_to.window(handles[index])
                time.sleep(1)
                print(json.dumps({'action': 'switch', 'tabs': [{'title': driver.title, 'url': driver.current_url, 'active': True}]}, indent=2))
            else:
                print(json.dumps({'action': 'switch', 'error': f'Tab index {index} out of range (have {len(handles)})'}))

        elif action == 'close':
            handles = driver.window_handles
            if index < len(handles) and len(handles) > 1:
                driver.switch_to.window(handles[index])
                driver.close()
                driver.switch_to.window(driver.window_handles[0])
                tabs = []
                for i, handle in enumerate(driver.window_handles):
                    driver.switch_to.window(handle)
                    tabs.append({'title': driver.title, 'url': driver.current_url, 'active': i == 0})
                print(json.dumps({'action': 'close', 'tabs': tabs}, indent=2))
            else:
                print(json.dumps({'action': 'close', 'error': 'Cannot close: invalid index or last tab'}))
    finally:
        driver.quit()


def cmd_wait():
    """Wait for element or text to appear."""
    if not wait_for:
        print('Usage: ghost.py wait <url> --for "selector|text" [--timeout 30]', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    try:
        from selenium.webdriver.common.by import By

        print(f'[ghost] Navigating to {url}, waiting for "{wait_for}"...', file=sys.stderr)
        driver.get(url)
        start = time.time()

        found = False
        element_text = ''
        while (time.time() - start) < timeout:
            try:
                el = driver.find_element(By.CSS_SELECTOR, wait_for)
                if el and el.is_displayed():
                    found = True
                    element_text = el.text
                    break
            except Exception:
                pass

            try:
                body_text = driver.find_element(By.TAG_NAME, 'body').text
                if wait_for in body_text:
                    found = True
                    element_text = wait_for
                    break
            except Exception:
                pass

            time.sleep(0.5)

        elapsed_ms = int((time.time() - start) * 1000)
        print(f'[ghost] {"Found" if found else "Not found"} after {elapsed_ms}ms', file=sys.stderr)
        print(json.dumps({'found': found, 'elapsed_ms': elapsed_ms, 'element_text': element_text}))
    finally:
        driver.quit()


def cmd_pipeline():
    """Execute multiple ghost actions in sequence with a shared driver."""
    if not steps_json:
        print('Usage: ghost.py pipeline --steps \'[{"action":"navigate","url":"..."}]\' [--stop-on-error]', file=sys.stderr)
        sys.exit(1)

    try:
        steps = json.loads(steps_json)
    except json.JSONDecodeError as e:
        print(f'[ghost] Invalid JSON in --steps: {e}', file=sys.stderr)
        sys.exit(1)

    driver = create_driver()
    results = []
    try:
        from selenium.webdriver.common.by import By
        from selenium.webdriver.common.keys import Keys
        from selenium.webdriver.support.ui import WebDriverWait
        from selenium.webdriver.support import expected_conditions as EC

        for i, step in enumerate(steps):
            act = step.get('action', '')
            print(f'[ghost] Step {i+1}/{len(steps)}: {act}', file=sys.stderr)
            result = {'action': act, 'success': False, 'result': None}

            try:
                if act == 'navigate':
                    step_url = step.get('url', '')
                    driver.get(step_url)
                    time.sleep(step.get('wait', 3))
                    result.update({'success': True, 'result': {'title': driver.title, 'url': driver.current_url}})

                elif act == 'click':
                    text = step.get('text', '')
                    sel = step.get('selector', '')
                    if sel:
                        el = WebDriverWait(driver, 10).until(EC.element_to_be_clickable((By.CSS_SELECTOR, sel)))
                    else:
                        el = driver.find_element(By.XPATH, f'//*[contains(text(),"{text}")]')
                    el.click()
                    time.sleep(step.get('wait', 1))
                    result.update({'success': True, 'result': {'clicked': text or sel}})

                elif act == 'type':
                    sel = step.get('selector', 'input')
                    val = step.get('value', '')
                    el = WebDriverWait(driver, 10).until(EC.presence_of_element_located((By.CSS_SELECTOR, sel)))
                    el.clear()
                    el.send_keys(val)
                    if step.get('submit'):
                        el.send_keys(Keys.RETURN)
                    time.sleep(step.get('wait', 1))
                    result.update({'success': True, 'result': {'typed': val, 'selector': sel}})

                elif act == 'screenshot':
                    path = step.get('output', f'/tmp/ghost-pipeline-{i}.png')
                    driver.save_screenshot(path)
                    result.update({'success': True, 'result': {'path': path}})

                elif act == 'extract':
                    sel = step.get('selector', 'body')
                    el = driver.find_element(By.CSS_SELECTOR, sel)
                    result.update({'success': True, 'result': {'text': el.text[:1000], 'selector': sel}})

                elif act == 'wait':
                    target = step.get('for', step.get('selector', ''))
                    wait_timeout = step.get('timeout', 10)
                    WebDriverWait(driver, wait_timeout).until(
                        EC.presence_of_element_located((By.CSS_SELECTOR, target))
                    )
                    result.update({'success': True, 'result': {'found': target}})

                elif act == 'scroll':
                    px = step.get('amount', 500)
                    d = step.get('direction', 'down')
                    scroll_val = px if d == 'down' else -px
                    driver.execute_script(f'window.scrollBy(0, {scroll_val});')
                    time.sleep(step.get('wait', 1))
                    result.update({'success': True, 'result': {'scrolled': scroll_val}})

                elif act == 'script':
                    code = step.get('code', '')
                    ret = driver.execute_script(code)
                    result.update({'success': True, 'result': {'return': ret}})

                else:
                    result.update({'success': False, 'result': f'Unknown action: {act}'})

            except Exception as e:
                result.update({'success': False, 'result': str(e)})
                print(f'[ghost] Step {i+1} failed: {e}', file=sys.stderr)
                if stop_on_error:
                    results.append(result)
                    break

            results.append(result)

        print(json.dumps(results, indent=2))
    finally:
        driver.quit()
