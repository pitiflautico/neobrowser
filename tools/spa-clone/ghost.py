#!/usr/bin/env python3
"""
Ghost Browser — undetectable Chrome with persistent sessions.

Uses undetected-chromedriver to bypass Cloudflare/bot detection.

Usage:
  python3 ghost.py test                                    Bot detection test
  python3 ghost.py open <url> [--profile "Profile 24"]     Open URL with cookies
  python3 ghost.py pong <url> --message "hi" [--profile P] Send chat message
  python3 ghost.py screenshot <url> [--output file.png]    Screenshot

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


# ── Main ──
if command == 'test':
    cmd_test()
elif command == 'open' and url:
    cmd_open()
elif command == 'pong' and url:
    cmd_pong()
else:
    print(__doc__)
