#!/usr/bin/env python3
"""
NeoV3 — Unified AI Browser. Best of V1 + V2.

Architecture:
  FAST PATH (V1 neobrowser): browse, search, read, observe — 500ms-3s
  CHROME PATH (neomode): click, type, fill_form, login, chat, screenshot — 5-15s
  AUTO: tries fast path first, falls back to chrome if empty

One MCP server. One tool interface. Best engine auto-selected.
"""

import json, sys, os, time, subprocess, threading, atexit, signal
from pathlib import Path

def log(msg):
    print(f'[v3] {msg}', file=sys.stderr, flush=True)

# ── V1 engine (fast HTTP + Chrome fallback) ──

def v1_call(command, url, extra_args=None):
    """Call neobrowser V1 CLI. Returns (output_string, elapsed_ms)."""
    args = ['neobrowser', command, url]
    if extra_args:
        args.extend(extra_args)
    start = time.time()
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=30)
        ms = int((time.time() - start) * 1000)
        return r.stdout.strip(), ms
    except subprocess.TimeoutExpired:
        return '', 30000
    except Exception as e:
        return f'Error: {e}', 0

def v1_search(query, num=10):
    """V1 DuckDuckGo search."""
    args = ['neobrowser', 'search', query, '--num', str(num)]
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=15)
        return r.stdout.strip()
    except:
        return ''

# ── Chrome engine (neomode headless) ──

_chrome_driver = None
_chrome_lock = threading.Lock()
_chrome_failed = False
_chrome_pids = set()
_chrome_tabs = {}

PID_FILE = Path.home() / '.neorender' / 'v3-pids.json'
RESPONSE_DIR = Path.home() / '.neorender' / 'ai-chat-responses'
RESPONSE_DIR.mkdir(parents=True, exist_ok=True)

NEOMODE_JS = '''
Object.defineProperty(screen, 'width', {get: () => 1920});
Object.defineProperty(screen, 'height', {get: () => 1080});
Object.defineProperty(screen, 'availWidth', {get: () => 1920});
Object.defineProperty(screen, 'availHeight', {get: () => 1055});
Object.defineProperty(window, 'outerHeight', {get: () => 1055});
Object.defineProperty(window, 'innerHeight', {get: () => 968});
'''
CHROME_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
PROFILE = os.environ.get('NEOBROWSER_PROFILE', 'Profile 24')

def _kill_stale():
    try:
        if PID_FILE.exists():
            for pid in json.loads(PID_FILE.read_text()):
                try: os.kill(int(pid), 9)
                except: pass
            PID_FILE.unlink(missing_ok=True)
            time.sleep(1)
    except: pass

_kill_stale()

def ensure_chrome():
    """Lazy singleton Chrome — only launched when needed."""
    global _chrome_driver, _chrome_failed

    if _chrome_driver:
        try:
            _ = _chrome_driver.title
            return _chrome_driver
        except:
            log('Chrome died, cleaning up...')
            try: _chrome_driver.quit()
            except: pass
            for pid in list(_chrome_pids):
                try: os.kill(pid, 9)
                except: pass
            _chrome_pids.clear()
            _chrome_driver = None
            _chrome_tabs.clear()
            _chrome_failed = False
            time.sleep(1)

    if _chrome_failed:
        raise RuntimeError('Chrome launch failed. Restart to retry.')

    if not _chrome_lock.acquire(blocking=False):
        _chrome_lock.acquire()
        _chrome_lock.release()
        if _chrome_driver: return _chrome_driver
        raise RuntimeError('Chrome launch in progress')

    try:
        import undetected_chromedriver.patcher as patcher
        from selenium import webdriver
        from selenium.webdriver.chrome.service import Service

        log('Launching Chrome (neomode)...')
        pa = patcher.Patcher(version_main=146)
        pa.auto()

        options = webdriver.ChromeOptions()
        options.add_argument('--window-size=1920,1080')
        options.add_argument('--no-sandbox')
        options.add_argument('--disable-dev-shm-usage')
        options.add_argument('--disable-blink-features=AutomationControlled')
        options.add_argument(f'--user-agent={CHROME_UA}')
        options.add_argument('--headless=new')
        options.binary_location = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
        options.add_experimental_option('excludeSwitches', ['enable-automation'])

        svc = Service(pa.executable_path)
        _chrome_driver = webdriver.Chrome(service=svc, options=options)

        if hasattr(_chrome_driver, 'service') and hasattr(_chrome_driver.service, 'process'):
            _chrome_pids.add(_chrome_driver.service.process.pid)
        PID_FILE.parent.mkdir(parents=True, exist_ok=True)
        PID_FILE.write_text(json.dumps(list(_chrome_pids)))

        _chrome_driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})
        _chrome_tabs['main'] = _chrome_driver.current_window_handle
        log(f'Chrome ready (pids={_chrome_pids})')
    except Exception as e:
        _chrome_failed = True
        log(f'Chrome FAILED: {e}')
        raise
    finally:
        _chrome_lock.release()

    return _chrome_driver

def chrome_navigate(url, wait_s=5):
    d = ensure_chrome()
    d.get(url)
    time.sleep(wait_s)
    return d

def import_cookies(d, domain):
    script = Path(__file__).parent.parent / 'spa-clone' / 'import-cookies.mjs'
    if not script.exists(): return
    try:
        r = subprocess.run(['node', str(script), domain, '--profile', PROFILE, '--json'],
                           capture_output=True, text=True, timeout=15)
        if r.returncode != 0: return
        cookies = json.loads(r.stdout)
        d.get(f'https://{domain}')
        time.sleep(1)
        ok = 0
        for c in cookies:
            try:
                cookie = {'name': c['name'], 'value': c['value']}
                if c.get('domain'): cookie['domain'] = c['domain']
                if c.get('path'): cookie['path'] = c['path']
                if c.get('secure'): cookie['secure'] = c['secure']
                if c.get('http_only'): cookie['httpOnly'] = c['http_only']
                d.add_cookie(cookie)
                ok += 1
            except: pass
        log(f'Imported {ok}/{len(cookies)} cookies for {domain}')
    except: pass


# ── Unified actions ──

def action_browse(args):
    """FAST: V1 browse. Falls back to Chrome if empty."""
    url = args.get('url', '')
    if not url: return {'error': 'url required'}
    out, ms = v1_call('see', url)
    if len(out) > 200:
        log(f'V1 browse: {ms}ms, {len(out)} chars')
        return out
    # Fallback to Chrome
    log(f'V1 empty ({len(out)} chars), using Chrome...')
    d = chrome_navigate(url)
    text = d.execute_script('return document.body?.innerText?.substring(0,2000)') or ''
    title = d.title
    return f'{title} | {url}\n\n{text}'

def action_search(args):
    """FAST: V1 search."""
    query = args.get('query', '')
    if not query: return {'error': 'query required'}
    num = int(args.get('num', 10))
    out = v1_search(query, num)
    if out:
        # Parse V1 JSON output into compact text
        try:
            data = json.loads(out)
            results = data.get('results', [])
            lines = []
            for i, r in enumerate(results[:num]):
                t = r.get('title', '')
                u = r.get('url', '')
                s = r.get('snippet', '')
                domain = u.replace('https://','').replace('http://','').split('/')[0]
                lines.append(f'{i+1}. {t} ({domain})')
                if s: lines.append(f'   {s[:150]}')
            return '\n'.join(lines)
        except:
            return out
    return 'No results'

def action_read(args):
    """FAST: V1 fetch + parse. Falls back to Chrome."""
    url = args.get('url', '')
    if not url: return {'error': 'url required'}
    out, ms = v1_call('fetch', url)
    if len(out) > 100:
        # Extract text content from V1 output
        lines = [l for l in out.split('\n') if l.strip() and not l.startswith('===')]
        text = '\n'.join(lines[:50])
        log(f'V1 read: {ms}ms, {len(text)} chars')
        return text
    # Fallback
    log('V1 read empty, using Chrome...')
    d = chrome_navigate(url, 3)
    js = '''
        const sels=['main','article','[role="main"]','#content','.content'];
        let el=null; for(const s of sels){el=document.querySelector(s);if(el)break}
        if(!el)el=document.body;
        const c=el.cloneNode(true);
        ['script','style','nav','footer','header','aside'].forEach(s=>c.querySelectorAll(s).forEach(n=>n.remove()));
        return {title:document.title, text:c.innerText.trim().substring(0,3000)};
    '''
    result = d.execute_script(js) or {}
    return f'{result.get("title","")} | {url}\n\n{result.get("text","")}'

def action_click(args):
    """CHROME: click element."""
    text = args.get('text', args.get('selector', ''))
    if not text: return {'error': 'text or selector required'}
    d = ensure_chrome()
    clicked = d.execute_script(f'''
        const q={json.dumps(text)};
        let els=document.querySelectorAll(q);
        if(!els.length){{
            const ql=q.toLowerCase();
            els=Array.from(document.querySelectorAll('a,button,[role="button"]')).filter(e=>
                (e.innerText||"").toLowerCase().includes(ql));
        }}
        if(els[0]){{els[0].click();return true}}
        return false;
    ''')
    time.sleep(2)
    return {'clicked': bool(clicked), 'url': d.current_url, 'title': d.title}

def action_type(args):
    """CHROME: type in input."""
    selector = args.get('selector', args.get('text', ''))
    value = args.get('value', '')
    if not selector or not value: return {'error': 'selector and value required'}
    d = ensure_chrome()
    from selenium.webdriver.common.by import By
    try:
        el = d.find_element(By.CSS_SELECTOR, selector)
    except:
        el = d.execute_script(f'''
            const q={json.dumps(selector)}.toLowerCase();
            return document.querySelector('[placeholder*="'+q+'"]')||
                   document.querySelector('[name*="'+q+'"]');
        ''')
    if not el: return {'error': f'Not found: {selector}'}
    el.clear()
    el.send_keys(value)
    return {'typed': True, 'value': value}

def action_fill_form(args):
    """CHROME: fill form fields."""
    url = args.get('url', '')
    fields = args.get('fields', '{}')
    if isinstance(fields, str): fields = json.loads(fields)
    d = ensure_chrome()
    if url: d.get(url); time.sleep(5)
    result = d.execute_script(f'''
        const fields={json.dumps(fields)};
        const filled=[],errors=[];
        for(const[key,val]of Object.entries(fields)){{
            const el=document.querySelector('[name="'+key+'"]')||
                     document.querySelector('[id="'+key+'"]')||
                     document.querySelector('[placeholder*="'+key+'" i]')||
                     document.querySelector('[type="'+key+'"]');
            if(el){{
                const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;
                if(s)s.call(el,val);else el.value=val;
                el.dispatchEvent(new Event('input',{{bubbles:true}}));
                el.dispatchEvent(new Event('change',{{bubbles:true}}));
                filled.push(key);
            }}else errors.push(key);
        }}
        return {{filled,errors}};
    ''')
    return result or {'filled': [], 'errors': list(fields.keys())}

def action_screenshot(args):
    """CHROME: screenshot."""
    url = args.get('url', '')
    d = ensure_chrome()
    if url: d.get(url); time.sleep(3)
    path = '/tmp/v3-screenshot.png'
    d.save_screenshot(path)
    return {'path': path}

def action_navigate(args):
    """CHROME: navigate and return info."""
    url = args.get('url', '')
    if not url: return {'error': 'url required'}
    d = chrome_navigate(url, int(args.get('wait', 5000)) / 1000)
    return {
        'title': d.title, 'url': d.current_url,
        'elements': d.execute_script('return document.querySelectorAll("*").length'),
    }

def action_chat_gpt(message, wait=True):
    """CHROME: ChatGPT."""
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys
    from selenium.webdriver.support.ui import WebDriverWait
    from selenium.webdriver.support import expected_conditions as EC

    d = ensure_chrome()
    if 'chatgpt' not in _chrome_tabs:
        d.switch_to.new_window('tab')
        _chrome_tabs['chatgpt'] = d.current_window_handle
        import_cookies(d, 'chatgpt.com')
        import_cookies(d, 'openai.com')
        d.get('https://chatgpt.com')
        time.sleep(8)
        log(f'ChatGPT ready: {d.title}')
    else:
        d.switch_to.window(_chrome_tabs['chatgpt'])

    el = WebDriverWait(d, 10).until(EC.presence_of_element_located((By.ID, 'prompt-textarea')))
    el.click(); time.sleep(0.3)
    el.send_keys(message); time.sleep(0.5)
    try: d.find_element(By.CSS_SELECTOR, '[data-testid="send-button"]').click()
    except: el.send_keys(Keys.RETURN)
    log('GPT: sent')

    if not wait: return 'Sent. Use status to check.'
    for i in range(120):
        time.sleep(1)
        streaming = d.execute_script('return !!document.querySelector("[data-testid=stop-button]")')
        if not streaming and i > 3:
            resp = d.execute_script('''
                const m=document.querySelectorAll('[data-message-author-role="assistant"]');
                return m.length?m[m.length-1].innerText:null;
            ''')
            if resp:
                log(f'GPT response: {len(resp)} chars')
                if len(resp) > 500:
                    ts = time.strftime('%Y%m%d-%H%M%S')
                    p = RESPONSE_DIR / f'gpt-{ts}.md'
                    p.write_text(resp)
                    return resp[:500] + f'...\n[Full: {len(resp)} chars → {p}]'
                return resp
    return 'No response after 120s'

def action_chat_grok(message, wait=True):
    """CHROME: Grok."""
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys

    d = ensure_chrome()
    if 'grok' not in _chrome_tabs:
        d.switch_to.new_window('tab')
        _chrome_tabs['grok'] = d.current_window_handle
        d.get('https://grok.com')
        time.sleep(8)
        log(f'Grok ready: {d.title}')
    else:
        d.switch_to.window(_chrome_tabs['grok'])

    el = d.find_element(By.CSS_SELECTOR, 'textarea, [contenteditable="true"], [role="textbox"]')
    el.send_keys(message); time.sleep(0.3)
    el.send_keys(Keys.RETURN)
    log('Grok: sent')

    if not wait: return 'Sent.'
    for i in range(60):
        time.sleep(1)
        if i > 5:
            resp = d.execute_script('''
                const b=document.querySelectorAll('div[class*="response"],div[class*="message-content"],article');
                return b.length>0?b[b.length-1].innerText:null;
            ''')
            if resp and len(resp) > 5: return resp
    return 'No response after 60s'


# ── Dispatch ──

def dispatch(action, args):
    FAST = {'browse': action_browse, 'search': action_search, 'read': action_read}
    CHROME = {
        'click': action_click, 'type': action_type, 'fill_form': action_fill_form,
        'screenshot': action_screenshot, 'navigate': action_navigate,
    }

    if action in FAST:
        return FAST[action](args)
    elif action in CHROME:
        return CHROME[action](args)
    elif action == 'open':
        return action_browse(args)
    else:
        return {'error': f'Unknown action: {action}'}


# ── Cleanup ──

def cleanup():
    global _chrome_driver
    if _chrome_driver:
        try: _chrome_driver.quit()
        except: pass
    for pid in _chrome_pids:
        try: os.kill(pid, 9)
        except: pass
    PID_FILE.unlink(missing_ok=True)
    log('Cleanup')

atexit.register(cleanup)
signal.signal(signal.SIGTERM, lambda *a: (cleanup(), sys.exit(0)))


# ── MCP ──

TOOLS = [
    {
        "name": "ghost",
        "description": "V3 AI browser. Auto-selects fastest engine. FAST (V1 HTTP ~1s): browse, search, read. CHROME (neomode ~10s): click, type, fill_form, screenshot, navigate. Use browse for reading pages, search for web search, click/type for interaction.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["browse","search","read","click","type","fill_form","screenshot","navigate","open"], "description": "Action"},
                "url": {"type": "string"}, "query": {"type": "string"},
                "text": {"type": "string"}, "selector": {"type": "string"},
                "value": {"type": "string"}, "fields": {"type": "string"},
                "num": {"type": "integer", "default": 10},
                "wait": {"type": "integer", "default": 5000},
            },
            "required": ["action"]
        }
    },
    {"name": "gpt", "description": "ChatGPT. Persistent conversation.", "inputSchema": {"type":"object","properties":{"message":{"type":"string"},"raw":{"type":"boolean","default":False},"wait":{"type":"boolean","default":True}},"required":["message"]}},
    {"name": "grok", "description": "Grok. Persistent conversation.", "inputSchema": {"type":"object","properties":{"message":{"type":"string"},"wait":{"type":"boolean","default":True}},"required":["message"]}},
    {"name": "ai_status", "description": "Status.", "inputSchema": {"type":"object","properties":{}}},
]

def respond(id, result):
    sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":id,"result":result})+'\n')
    sys.stdout.flush()

def respond_error(id, code, msg):
    sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})+'\n')
    sys.stdout.flush()

def handle(req):
    method = req.get('method','')
    params = req.get('params',{})
    id = req.get('id')

    if method == 'initialize':
        respond(id, {"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"neo-v3","version":"3.0.0"}})
    elif method == 'tools/list':
        respond(id, {"tools": TOOLS})
    elif method == 'tools/call':
        name = params.get('name','')
        args = params.get('arguments',{})
        try:
            if name == 'ghost':
                result = dispatch(args.get('action','browse'), args)
                text = json.dumps(result, indent=2, ensure_ascii=False) if not isinstance(result, str) else result
                respond(id, {"content":[{"type":"text","text":text}]})
            elif name == 'gpt':
                resp = action_chat_gpt(args.get('message',''), args.get('wait',True))
                respond(id, {"content":[{"type":"text","text":resp}]})
            elif name == 'grok':
                resp = action_chat_grok(args.get('message',''), args.get('wait',True))
                respond(id, {"content":[{"type":"text","text":resp}]})
            elif name == 'ai_status':
                respond(id, {"content":[{"type":"text","text":json.dumps({"chrome": _chrome_driver is not None, "tabs": list(_chrome_tabs.keys()), "pids": list(_chrome_pids)}, indent=2)}]})
            else:
                respond_error(id, -32601, f"Unknown: {name}")
        except Exception as e:
            respond(id, {"content":[{"type":"text","text":f"Error: {e}"}],"isError":True})
    elif method == 'notifications/initialized':
        pass
    elif id is not None:
        respond_error(id, -32601, f"Unknown: {method}")

log('V3 started — V1 fast + Chrome neomode')

for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: handle(json.loads(line))
    except json.JSONDecodeError: log(f'JSON err: {line[:80]}')
    except Exception as e: log(f'Error: {e}')
