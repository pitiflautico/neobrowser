#!/usr/bin/env python3
"""
Tests unitarios para el chat pipeline de neo-browser.
Cada test aísla un componente y verifica que funciona.
Ejecutar: python3 tests/test_chat_pipeline.py
"""

import json, sys, os, time, socket, subprocess, urllib.request
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'tools', 'v3'))

CHROME_BIN = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
PROFILE = os.environ.get('NEOBROWSER_PROFILE', 'Profile 24')

try:
    import websockets.sync.client as ws_sync
except ImportError:
    print('FAIL: pip install websockets')
    sys.exit(1)

# ── Helpers ──

def free_port():
    s = socket.socket(); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.close()
    return port

def launch_chrome(profile_dir, port):
    proc = subprocess.Popen([CHROME_BIN, f'--remote-debugging-port={port}',
        f'--user-data-dir={profile_dir}', '--no-first-run',
        '--disable-background-networking', '--disable-dev-shm-usage',
        '--disable-blink-features=AutomationControlled',
        '--window-size=1920,1080', '--window-position=-32000,-32000', f'--user-agent={UA}', 'about:blank'],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2)
    return proc

def cdp_connect(port):
    """Connect to the default page target."""
    targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
    page = [t for t in targets if t['type'] == 'page'][0]
    ws = ws_sync.connect(page['webSocketDebuggerUrl'], max_size=10_000_000, ping_interval=None)
    return ws

def cdp_send(ws, method, params=None, _state={'id': 0}):
    _state['id'] += 1
    ws.send(json.dumps({'id': _state['id'], 'method': method, 'params': params or {}}))
    while True:
        data = json.loads(ws.recv(timeout=30))
        if data.get('id') == _state['id']:
            return data.get('result', {})

def cdp_js(ws, code):
    expr = f'(function(){{{code}}})()' if 'return ' in code else code
    r = cdp_send(ws, 'Runtime.evaluate', {'expression': expr, 'returnByValue': True, 'awaitPromise': False})
    return r.get('result', {}).get('value')

def test(name, passed, detail=''):
    status = '✅' if passed else '❌'
    print(f'{status} {name}' + (f' — {detail}' if detail else ''))
    return passed

# ── Tests ──

def test_1_chrome_launches():
    """Chrome headless launches and accepts CDP connections."""
    import tempfile
    port = free_port()
    profile = tempfile.mkdtemp()
    proc = launch_chrome(profile, port)
    try:
        targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
        pages = [t for t in targets if t['type'] == 'page']
        return test('Chrome launches', len(pages) > 0, f'{len(pages)} page targets')
    except Exception as e:
        return test('Chrome launches', False, str(e))
    finally:
        proc.kill()

def test_2_default_tab_navigates():
    """Default tab navigates to a URL and loads content."""
    import tempfile
    port = free_port()
    proc = launch_chrome(tempfile.mkdtemp(), port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        cdp_send(ws, 'Page.navigate', {'url': 'https://example.com'})
        time.sleep(3)
        title = cdp_js(ws, 'return document.title')
        ws.close()
        return test('Default tab navigates', title == 'Example Domain', f'title="{title}"')
    finally:
        proc.kill()

def test_3_target_create_with_url():
    """Target.createTarget with URL — does the new tab navigate?"""
    import tempfile
    port = free_port()
    proc = launch_chrome(tempfile.mkdtemp(), port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        # Create new target WITH URL
        result = cdp_send(ws, 'Target.createTarget', {'url': 'https://example.com'})
        target_id = result.get('targetId', '')
        ok1 = test('Target.createTarget returns ID', bool(target_id), f'id={target_id}')

        time.sleep(3)
        # Connect to new target
        targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
        new_target = next((t for t in targets if t.get('id') == target_id), None)
        ok2 = test('New target in list', new_target is not None)

        if new_target:
            ws2 = ws_sync.connect(new_target['webSocketDebuggerUrl'], max_size=10_000_000, ping_interval=None)
            cdp_send(ws2, 'Page.enable')
            time.sleep(2)
            url = cdp_js(ws2, 'return location.href')
            title = cdp_js(ws2, 'return document.title')
            ok3 = test('New tab has correct URL', 'example.com' in (url or ''), f'url={url}')
            ok4 = test('New tab has content', title == 'Example Domain', f'title={title}')
            ws2.close()
        else:
            ok3 = test('New tab has correct URL', False, 'target not found')
            ok4 = test('New tab has content', False, 'target not found')

        ws.close()
        return ok1 and ok2 and ok3 and ok4
    finally:
        proc.kill()

def test_4_target_create_blank_then_navigate():
    """Target.createTarget(about:blank) then Page.navigate — does it work?"""
    import tempfile
    port = free_port()
    proc = launch_chrome(tempfile.mkdtemp(), port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        result = cdp_send(ws, 'Target.createTarget', {'url': 'about:blank'})
        target_id = result.get('targetId', '')

        time.sleep(1)
        targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
        new_target = next((t for t in targets if t.get('id') == target_id), None)

        if not new_target:
            return test('Blank+navigate', False, 'target not found')

        ws2 = ws_sync.connect(new_target['webSocketDebuggerUrl'], max_size=10_000_000, ping_interval=None)
        cdp_send(ws2, 'Page.enable')
        cdp_send(ws2, 'Page.navigate', {'url': 'https://example.com'})
        time.sleep(3)
        url = cdp_js(ws2, 'return location.href')
        title = cdp_js(ws2, 'return document.title')
        ok = test('Blank then navigate works', 'example.com' in (url or ''), f'url={url}, title={title}')
        ws2.close()
        ws.close()
        return ok
    finally:
        proc.kill()

def test_5_cookies_shared_across_tabs():
    """Cookies set in tab 1 are visible in tab 2."""
    import tempfile
    port = free_port()
    proc = launch_chrome(tempfile.mkdtemp(), port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        cdp_send(ws, 'Network.enable')
        # Set cookie via CDP
        cdp_send(ws, 'Network.setCookie', {
            'name': 'test_cookie', 'value': 'hello',
            'domain': 'example.com', 'path': '/',
            'url': 'https://example.com'
        })
        # Navigate tab 1 to example.com
        cdp_send(ws, 'Page.navigate', {'url': 'https://example.com'})
        time.sleep(2)
        cookie1 = cdp_js(ws, 'return document.cookie')

        # Create tab 2 and navigate
        result = cdp_send(ws, 'Target.createTarget', {'url': 'https://example.com'})
        target_id = result.get('targetId', '')
        time.sleep(2)
        targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
        new_target = next((t for t in targets if t.get('id') == target_id), None)

        if not new_target:
            return test('Cookies shared', False, 'tab 2 not created')

        ws2 = ws_sync.connect(new_target['webSocketDebuggerUrl'], max_size=10_000_000, ping_interval=None)
        cdp_send(ws2, 'Page.enable')
        time.sleep(2)
        cookie2 = cdp_js(ws2, 'return document.cookie')
        ws2.close()
        ws.close()
        return test('Cookies shared across tabs', 'test_cookie' in (cookie2 or ''),
                     f'tab1="{cookie1}", tab2="{cookie2}"')
    finally:
        proc.kill()

def test_6_script_injection_on_new_tab():
    """addScriptToEvaluateOnNewDocument on new tab — does it execute?"""
    import tempfile
    port = free_port()
    proc = launch_chrome(tempfile.mkdtemp(), port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        result = cdp_send(ws, 'Target.createTarget', {'url': 'about:blank'})
        target_id = result.get('targetId', '')
        time.sleep(1)

        targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=5).read())
        new_target = next((t for t in targets if t.get('id') == target_id), None)
        ws2 = ws_sync.connect(new_target['webSocketDebuggerUrl'], max_size=10_000_000, ping_interval=None)
        cdp_send(ws2, 'Page.enable')
        # Inject script for FUTURE navigations
        cdp_send(ws2, 'Page.addScriptToEvaluateOnNewDocument', {'source': 'window.__TEST_INJECT = 42;'})
        # Navigate — script should run
        cdp_send(ws2, 'Page.navigate', {'url': 'https://example.com'})
        time.sleep(3)
        val = cdp_js(ws2, 'return window.__TEST_INJECT')
        ok = test('Script injection on new tab', val == 42, f'__TEST_INJECT={val}')

        # Also test manual injection on current page (no navigation)
        cdp_js(ws2, 'window.__MANUAL = 99')
        val2 = cdp_js(ws2, 'return window.__MANUAL')
        test('Manual JS injection works', val2 == 99, f'__MANUAL={val2}')

        ws2.close(); ws.close()
        return ok
    finally:
        proc.kill()

def test_7_chatgpt_with_cookies():
    """ChatGPT loads with session from real Chrome cookies."""
    import tempfile, sqlite3, shutil
    ghost = tempfile.mkdtemp()
    ghost_default = os.path.join(ghost, 'Default')
    os.makedirs(ghost_default, exist_ok=True)

    # Sync cookies from real Chrome
    real_cookies = os.path.expanduser(f'~/Library/Application Support/Google/Chrome/{PROFILE}/Cookies')
    if not os.path.exists(real_cookies):
        return test('ChatGPT with cookies', False, f'No cookies at {real_cookies}')

    dst = os.path.join(ghost_default, 'Cookies')
    try:
        conn_src = sqlite3.connect(f'file:{real_cookies}?mode=ro&nolock=1', uri=True)
        conn_dst = sqlite3.connect(dst)
        conn_src.backup(conn_dst)
        # Remove Google cookies
        conn_dst.execute("DELETE FROM cookies WHERE host_key LIKE '%.google.com' OR host_key LIKE '%.googleapis.com'")
        count = conn_dst.execute('SELECT COUNT(*) FROM cookies').fetchone()[0]
        conn_dst.commit(); conn_dst.close(); conn_src.close()
    except Exception as e:
        return test('ChatGPT with cookies', False, f'Cookie sync failed: {e}')

    # Also sync Local Storage
    for dirname in ['Local Storage', 'Session Storage']:
        src = os.path.expanduser(f'~/Library/Application Support/Google/Chrome/{PROFILE}/{dirname}')
        if os.path.exists(src):
            shutil.copytree(src, os.path.join(ghost_default, dirname), dirs_exist_ok=True)

    port = free_port()
    proc = launch_chrome(ghost, port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        cdp_send(ws, 'Page.navigate', {'url': 'https://chatgpt.com'})
        time.sleep(8)
        title = cdp_js(ws, 'return document.title')
        has_textarea = cdp_js(ws, 'return !!document.getElementById("prompt-textarea")')
        url = cdp_js(ws, 'return location.href')
        logged_in = 'login' not in (url or '').lower()
        ws.close()
        return test('ChatGPT loads with session',
                     has_textarea and logged_in,
                     f'title="{title}", textarea={has_textarea}, logged_in={logged_in}, url={url}')
    finally:
        proc.kill()

def test_8_neoFind_on_chatgpt():
    """__neoFind detects ChatGPT's ProseMirror textarea."""
    import tempfile, sqlite3, shutil
    ghost = tempfile.mkdtemp()
    ghost_default = os.path.join(ghost, 'Default')
    os.makedirs(ghost_default, exist_ok=True)

    real_cookies = os.path.expanduser(f'~/Library/Application Support/Google/Chrome/{PROFILE}/Cookies')
    if not os.path.exists(real_cookies):
        return test('__neoFind on ChatGPT', False, 'No cookies')
    try:
        conn_src = sqlite3.connect(f'file:{real_cookies}?mode=ro&nolock=1', uri=True)
        conn_dst = sqlite3.connect(os.path.join(ghost_default, 'Cookies'))
        conn_src.backup(conn_dst)
        conn_dst.execute("DELETE FROM cookies WHERE host_key LIKE '%.google.com'")
        conn_dst.commit(); conn_dst.close(); conn_src.close()
    except:
        return test('__neoFind on ChatGPT', False, 'Cookie sync failed')

    for d in ['Local Storage', 'Session Storage']:
        src = os.path.expanduser(f'~/Library/Application Support/Google/Chrome/{PROFILE}/{d}')
        if os.path.exists(src):
            shutil.copytree(src, os.path.join(ghost_default, d), dirs_exist_ok=True)

    # Read NEOMODE_JS from neo-browser.py
    neo_path = os.path.join(os.path.dirname(__file__), '..', 'tools', 'v3', 'neo-browser.py')
    with open(neo_path) as f:
        content = f.read()
    # Extract NEOMODE_JS
    start = content.index("NEOMODE_JS = '''") + len("NEOMODE_JS = '''")
    end = content.index("'''", start)
    neomode_js = content[start:end]

    port = free_port()
    proc = launch_chrome(ghost, port)
    try:
        ws = cdp_connect(port)
        cdp_send(ws, 'Page.enable')
        cdp_send(ws, 'Page.navigate', {'url': 'https://chatgpt.com'})
        time.sleep(8)

        # Inject NEOMODE_JS manually
        cdp_js(ws, neomode_js)

        # Test __neoFind
        has_fn = cdp_js(ws, 'return typeof window.__neoFind')
        result = cdp_js(ws, '''
            const el = window.__neoFind?.();
            if (!el) return JSON.stringify({found: false});
            return JSON.stringify({found: true, tag: el.tagName, id: el.id, editable: el.isContentEditable});
        ''')
        try:
            info = json.loads(result)
        except:
            info = {'found': False, 'raw': result}

        ws.close()
        return test('__neoFind on ChatGPT',
                     info.get('found') and info.get('id') == 'prompt-textarea',
                     f'fn={has_fn}, result={info}')
    finally:
        proc.kill()


# ── Run ──

if __name__ == '__main__':
    print('=' * 60)
    print('NeoBrowser Chat Pipeline — Unit Tests')
    print('=' * 60)
    print()

    results = []
    results.append(test_1_chrome_launches())
    results.append(test_2_default_tab_navigates())
    results.append(test_3_target_create_with_url())
    results.append(test_4_target_create_blank_then_navigate())
    results.append(test_5_cookies_shared_across_tabs())
    results.append(test_6_script_injection_on_new_tab())
    results.append(test_7_chatgpt_with_cookies())
    results.append(test_8_neoFind_on_chatgpt())

    print()
    passed = sum(results)
    total = len(results)
    print(f'Results: {passed}/{total} passed')

    # Cleanup
    subprocess.run(['pkill', '-f', 'headless.*ghost'], capture_output=True)
