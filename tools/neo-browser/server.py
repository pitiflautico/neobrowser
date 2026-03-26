#!/usr/bin/env python3
"""
NeoBrowser MCP Server — persistent Chrome with tabs, neomode, all actions.

ONE Chrome instance. Multiple tabs. Stays alive between calls.
All ghost actions + ai-chat in a single browser.

MCP tools:
  ghost(action, params)  — All browser actions (search, navigate, read, click, etc.)
  gpt(message)           — Chat with ChatGPT (dedicated tab)
  grok(message)          — Chat with Grok (dedicated tab)
  ai_status()            — Check chat sessions
"""

import json, sys, os, time, atexit, signal, threading
from pathlib import Path
from urllib.parse import quote_plus

def log(msg):
    print(f'[neo-browser] {msg}', file=sys.stderr, flush=True)

# ── Chrome singleton ──

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
PID_FILE = Path.home() / '.neorender' / 'neo-browser-pids.json'
RESPONSE_DIR = Path.home() / '.neorender' / 'ai-chat-responses'
RESPONSE_DIR.mkdir(parents=True, exist_ok=True)

driver = None          # Single Chrome instance
tabs = {}              # name → {handle, url, title}
current_tab = None     # Active tab name
our_pids = set()


def _kill_stale():
    """Kill Chrome from previous crashed session (our PIDs only)."""
    try:
        if PID_FILE.exists():
            for pid in json.loads(PID_FILE.read_text()):
                try: os.kill(int(pid), 9)
                except: pass
            PID_FILE.unlink(missing_ok=True)
            time.sleep(1)
    except: pass

_kill_stale()

# Launch lock — prevents multiple simultaneous Chrome launches
_launch_lock = threading.Lock()
_launch_failed = False  # If True, don't retry until next restart


def ensure_browser():
    """Get or create the singleton Chrome browser. Thread-safe, single instance."""
    global driver, _launch_failed

    # Fast path: already running
    if driver:
        try:
            _ = driver.title
            return driver
        except:
            log('Chrome died, will recreate on next call')
            driver = None
            _launch_failed = False

    # Don't retry if already failed this session
    if _launch_failed:
        raise RuntimeError('Chrome launch failed earlier. Restart MCP server to retry.')

    # Only one thread can launch
    if not _launch_lock.acquire(blocking=False):
        # Another thread is launching — wait for it
        _launch_lock.acquire()
        _launch_lock.release()
        if driver:
            return driver
        raise RuntimeError('Chrome launch in progress')

    try:
        _launch_browser()
    except Exception as e:
        _launch_failed = True
        log(f'Chrome launch FAILED: {e}')
        raise
    finally:
        _launch_lock.release()

    return driver


def _launch_browser():
    """Actually launch Chrome. Called only once, under lock."""
    global driver

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
    driver = webdriver.Chrome(service=svc, options=options)

    # Track PIDs
    if hasattr(driver, 'service') and hasattr(driver.service, 'process'):
        our_pids.add(driver.service.process.pid)
    try:
        PID_FILE.parent.mkdir(parents=True, exist_ok=True)
        PID_FILE.write_text(json.dumps(list(our_pids)))
    except: pass

    # Neomode patches
    driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})

    # Register initial tab
    tabs['main'] = {'handle': driver.current_window_handle, 'url': '', 'title': ''}

    log(f'Chrome started (neomode headless, pids={our_pids})')
    return driver


# ── Tab management ──

def tab_new(name, url=None):
    """Open a new tab."""
    global current_tab
    d = ensure_browser()
    d.switch_to.new_window('tab')
    handle = d.current_window_handle
    tabs[name] = {'handle': handle, 'url': '', 'title': ''}
    current_tab = name
    if url:
        d.get(url)
        time.sleep(3)
        tabs[name]['url'] = d.current_url
        tabs[name]['title'] = d.title
    log(f'Tab "{name}" opened')
    return tabs[name]

def tab_switch(name):
    """Switch to a named tab."""
    global current_tab
    if name not in tabs:
        return None
    d = ensure_browser()
    d.switch_to.window(tabs[name]['handle'])
    current_tab = name
    tabs[name]['url'] = d.current_url
    tabs[name]['title'] = d.title
    return tabs[name]

def tab_list():
    """List all tabs."""
    d = ensure_browser()
    current_handle = d.current_window_handle
    result = []
    for name, info in tabs.items():
        result.append({
            'name': name,
            'url': info.get('url', ''),
            'title': info.get('title', ''),
            'active': info['handle'] == current_handle,
        })
    return result

def tab_close(name):
    """Close a named tab."""
    global current_tab
    if name not in tabs or name == 'main':
        return False
    d = ensure_browser()
    d.switch_to.window(tabs[name]['handle'])
    d.close()
    del tabs[name]
    # Switch back to main
    if tabs:
        first = next(iter(tabs))
        d.switch_to.window(tabs[first]['handle'])
        current_tab = first
    return True

def ensure_tab(name='main'):
    """Ensure we're on the right tab, create if needed."""
    global current_tab
    d = ensure_browser()
    if name in tabs:
        if current_tab != name:
            d.switch_to.window(tabs[name]['handle'])
            current_tab = name
    else:
        tab_new(name)
    return d


# ── Cookie import ──

def import_cookies(d, domain):
    """Import cookies from Chrome profile."""
    script = Path(__file__).parent.parent / 'spa-clone' / 'import-cookies.mjs'
    if not script.exists(): return

    try:
        import subprocess
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
    except Exception as e:
        log(f'Cookie import failed: {e}')


# ── Ghost actions ──

def action_search(args):
    query = args.get('query', '')
    engine = args.get('engine', 'duckduckgo')
    num = int(args.get('num', 10))
    if not query: return {'error': 'query required'}

    d = ensure_tab('search')
    if engine in ('google', 'g'):
        d.get(f'https://www.google.com/search?q={quote_plus(query)}&num={num}')
    else:
        d.get(f'https://html.duckduckgo.com/html/?q={quote_plus(query)}')
    time.sleep(3)

    if engine in ('google', 'g'):
        results = d.execute_script('''
            const out = [];
            document.querySelectorAll('div.g').forEach(el => {
                const h3 = el.querySelector('h3');
                const a = el.querySelector('a');
                const sn = el.querySelector('.VwiC3b, [data-sncf]');
                if (h3 && a) out.push({title: h3.innerText, url: a.href, snippet: sn?.innerText || ''});
            });
            return out;
        ''')
    else:
        results = d.execute_script('''
            const out = [];
            document.querySelectorAll('.result').forEach(el => {
                const a = el.querySelector('.result__title a, .result__a');
                const sn = el.querySelector('.result__snippet');
                if (a) out.push({title: a.innerText, url: a.href, snippet: sn?.innerText || ''});
            });
            return out;
        ''')

    return (results or [])[:num]

def action_navigate(args):
    url = args.get('url', '')
    if not url: return {'error': 'url required'}
    tab_name = args.get('tab', 'main')
    d = ensure_tab(tab_name)
    d.get(url)
    time.sleep(int(args.get('wait', 5000)) / 1000)
    tabs[tab_name]['url'] = d.current_url
    tabs[tab_name]['title'] = d.title
    return {
        'title': d.title, 'url': d.current_url,
        'elements': d.execute_script('return document.querySelectorAll("*").length'),
    }

def action_read(args):
    url = args.get('url', '')
    sel = args.get('selector', '')
    tab_name = args.get('tab', 'main')
    d = ensure_tab(tab_name)
    if url: d.get(url); time.sleep(3)

    if sel:
        js = f'const el=document.querySelector({json.dumps(sel)});return el?{{title:document.title,text:el.innerText.trim(),word_count:el.innerText.trim().split(/\\s+/).length}}:null'
    else:
        js = '''
            const sels=['main','article','[role="main"]','#content','.content'];
            let el=null; for(const s of sels){el=document.querySelector(s);if(el)break}
            if(!el)el=document.body;
            const c=el.cloneNode(true);
            ['script','style','nav','footer','header','aside','iframe','noscript'].forEach(s=>c.querySelectorAll(s).forEach(n=>n.remove()));
            const t=c.innerText.trim();
            return {title:document.title,text:t,word_count:t.split(/\\s+/).length};
        '''
    return d.execute_script(js) or {'title': '', 'text': '', 'word_count': 0}

def action_find(args):
    text = args.get('text', '')
    by = args.get('by', 'text')
    d = ensure_browser()

    if by == 'css':
        els = d.execute_script(f'return Array.from(document.querySelectorAll({json.dumps(text)})).slice(0,5).map(e=>({{tag:e.tagName,text:e.innerText?.substring(0,100),clickable:e.tagName==="A"||e.tagName==="BUTTON"||e.onclick!=null}}))')
    elif by == 'xpath':
        from selenium.webdriver.common.by import By
        found = d.find_elements(By.XPATH, text)[:5]
        els = [{'tag': e.tag_name, 'text': e.text[:100], 'clickable': e.tag_name in ('a','button')} for e in found]
    else:
        els = d.execute_script(f'''
            const q={json.dumps(text)}.toLowerCase();
            const all=document.querySelectorAll('a,button,input,select,textarea,h1,h2,h3,h4,label,[role]');
            const out=[];
            for(const e of all){{
                const t=(e.innerText||e.value||e.placeholder||e.getAttribute("aria-label")||"").toLowerCase();
                if(t.includes(q))out.push({{tag:e.tagName,text:e.innerText?.substring(0,100)||"",clickable:e.tagName==="A"||e.tagName==="BUTTON"}});
                if(out.length>=5)break;
            }}
            return out;
        ''')
    return {'found': len(els or []) > 0, 'elements': els or [], 'query': text}

def action_click(args):
    text = args.get('text', args.get('selector', ''))
    if not text: return {'error': 'text or selector required'}
    d = ensure_browser()
    idx = int(args.get('index', 0))

    clicked = d.execute_script(f'''
        const q={json.dumps(text)};
        let els=document.querySelectorAll(q);
        if(!els.length){{
            const ql=q.toLowerCase();
            els=Array.from(document.querySelectorAll('a,button,[role="button"],[onclick]')).filter(e=>
                (e.innerText||"").toLowerCase().includes(ql));
        }}
        if(els[{idx}]){{els[{idx}].click();return true}}
        return false;
    ''')
    time.sleep(2)
    return {'clicked': bool(clicked), 'url': d.current_url, 'title': d.title}

def action_type(args):
    selector = args.get('selector', args.get('text', ''))
    value = args.get('value', '')
    if not selector or not value: return {'error': 'selector and value required'}
    d = ensure_browser()
    from selenium.webdriver.common.by import By
    try:
        el = d.find_element(By.CSS_SELECTOR, selector)
    except:
        # Try by placeholder/label
        el = d.execute_script(f'''
            const q={json.dumps(selector)}.toLowerCase();
            return document.querySelector('[placeholder*="'+q+'"]')||
                   document.querySelector('[name*="'+q+'"]')||
                   document.querySelector('[aria-label*="'+q+'"]');
        ''')
        if not el: return {'error': f'Element not found: {selector}'}
    el.clear()
    el.send_keys(value)
    return {'typed': True, 'value': value}

def action_fill_form(args):
    url = args.get('url', '')
    fields = args.get('fields', '{}')
    if isinstance(fields, str): fields = json.loads(fields)
    d = ensure_browser()
    if url: d.get(url); time.sleep(5)

    filled = d.execute_script(f'''
        const fields={json.dumps(fields)};
        const filled=[]; const errors=[];
        for(const[key,val]of Object.entries(fields)){{
            const el=document.querySelector('[name="'+key+'"]')||
                     document.querySelector('[id="'+key+'"]')||
                     document.querySelector('[placeholder*="'+key+'" i]')||
                     document.querySelector('[type="'+key+'"]')||
                     document.querySelector('[aria-label*="'+key+'" i]');
            if(el){{
                const nativeSetter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set||
                                   Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value')?.set;
                if(nativeSetter)nativeSetter.call(el,val);
                else el.value=val;
                el.dispatchEvent(new Event('input',{{bubbles:true}}));
                el.dispatchEvent(new Event('change',{{bubbles:true}}));
                filled.push(key);
            }}else errors.push(key);
        }}
        return {{filled,errors}};
    ''')
    return filled or {'filled': [], 'errors': list(fields.keys())}

def action_submit(args):
    d = ensure_browser()
    btn_text = args.get('text', args.get('selector', ''))
    submitted = d.execute_script(f'''
        const q={json.dumps(btn_text)};
        let btn=q?document.querySelector(q):null;
        if(!btn)btn=document.querySelector('[type="submit"]');
        if(!btn)btn=document.querySelector('button');
        if(!btn){{const f=document.querySelector('form');if(f){{f.submit();return true}}}}
        if(btn){{btn.click();return true}}
        return false;
    ''')
    time.sleep(3)
    return {'submitted': bool(submitted), 'url': d.current_url, 'title': d.title}

def action_screenshot(args):
    d = ensure_browser()
    url = args.get('url', '')
    if url: d.get(url); time.sleep(3)
    path = '/tmp/neo-browser-screenshot.png'
    d.save_screenshot(path)
    return {'path': path}

def action_scroll(args):
    d = ensure_browser()
    direction = args.get('direction', 'down')
    amount = int(args.get('amount', 500))
    px = amount if direction == 'down' else -amount
    d.execute_script(f'window.scrollBy(0,{px})')
    time.sleep(1)
    info = d.execute_script('return {scrollY:window.scrollY,pageHeight:document.body.scrollHeight,atBottom:window.scrollY+window.innerHeight>=document.body.scrollHeight-10}')
    return info

def action_extract_data(args):
    url = args.get('url', '')
    dtype = args.get('type_', args.get('type', 'links'))
    d = ensure_browser()
    if url: d.get(url); time.sleep(3)

    if dtype == 'table':
        return d.execute_script('''
            const t=document.querySelector('table');if(!t)return {data:[]};
            const hs=Array.from(t.querySelectorAll('th')).map(h=>h.innerText.trim());
            const rows=[];
            t.querySelectorAll('tr').forEach(r=>{
                const cells=Array.from(r.querySelectorAll('td')).map(c=>c.innerText.trim());
                if(cells.length){const obj={};cells.forEach((c,i)=>obj[hs[i]||'col'+i]=c);rows.push(obj)}
            });
            return {headers:hs,rows};
        ''')
    elif dtype == 'links':
        return d.execute_script('return Array.from(document.querySelectorAll("a[href]")).map(a=>({text:a.innerText.trim(),href:a.href})).filter(l=>l.text)')
    elif dtype == 'list':
        return d.execute_script('return Array.from(document.querySelectorAll("li")).map(l=>l.innerText.trim()).filter(t=>t)')
    return {'error': f'Unknown type: {dtype}'}

def action_wait_for(args):
    sel = args.get('selector', args.get('for', ''))
    timeout = int(args.get('timeout', 30))
    d = ensure_browser()
    start = time.time()
    for _ in range(timeout):
        found = d.execute_script(f'return !!document.querySelector({json.dumps(sel)})')
        if found:
            elapsed = int((time.time()-start)*1000)
            text = d.execute_script(f'return document.querySelector({json.dumps(sel)})?.innerText?.substring(0,200)')
            return {'found': True, 'elapsed_ms': elapsed, 'text': text}
        time.sleep(1)
    return {'found': False, 'elapsed_ms': timeout*1000}

def action_tabs(args):
    act = args.get('action', 'list')
    if act == 'list': return tab_list()
    elif act == 'new': return tab_new(args.get('name', f'tab-{len(tabs)}'), args.get('url'))
    elif act == 'switch': return tab_switch(args.get('name', 'main'))
    elif act == 'close': return {'closed': tab_close(args.get('name', ''))}
    return {'error': f'Unknown tab action: {act}'}

def action_pipeline(args):
    steps = args.get('steps', '[]')
    if isinstance(steps, str): steps = json.loads(steps)
    results = []
    for step in steps:
        act = step.get('action', '')
        result = dispatch_action(act, step)
        results.append({'action': act, 'success': 'error' not in str(result), 'result': result})
        if step.get('stop_on_error') and 'error' in str(result): break
    return results


# ── Chat actions (dedicated tabs) ──

chat_pending = {}

def action_chat_gpt(message, wait=True):
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys
    from selenium.webdriver.support.ui import WebDriverWait
    from selenium.webdriver.support import expected_conditions as EC

    d = ensure_tab('chatgpt')
    if not tabs['chatgpt'].get('ready'):
        import_cookies(d, 'chatgpt.com')
        import_cookies(d, 'openai.com')
        d.get('https://chatgpt.com')
        time.sleep(8)
        tabs['chatgpt']['ready'] = True
        log(f'ChatGPT ready: {d.title}')

    el = WebDriverWait(d, 10).until(EC.presence_of_element_located((By.ID, 'prompt-textarea')))
    el.click(); time.sleep(0.3)
    el.send_keys(message); time.sleep(0.5)
    try: d.find_element(By.CSS_SELECTOR, '[data-testid="send-button"]').click()
    except: el.send_keys(Keys.RETURN)
    log('GPT: message sent')

    if not wait: return 'Message sent. Use ai_status to check.'

    # Wait for response
    for i in range(120):
        time.sleep(1)
        streaming = d.execute_script('return !!document.querySelector("[data-testid=stop-button]")')
        if not streaming and i > 3:
            resp = d.execute_script('''
                const m=document.querySelectorAll('[data-message-author-role="assistant"]');
                return m.length?m[m.length-1].innerText:null;
            ''')
            if resp: return save_response('chatgpt', resp)
    return 'Error: No response'

def action_chat_grok(message, wait=True):
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys

    d = ensure_tab('grok')
    if not tabs['grok'].get('ready'):
        d.get('https://grok.com')
        time.sleep(10)
        # Wait for textarea to appear
        from selenium.webdriver.common.by import By
        from selenium.webdriver.support.ui import WebDriverWait
        from selenium.webdriver.support import expected_conditions as EC
        try:
            WebDriverWait(d, 15).until(
                EC.presence_of_element_located((By.CSS_SELECTOR, 'textarea, [contenteditable="true"], [role="textbox"]'))
            )
        except:
            log(f'Grok: textarea not found after 15s. Title: {d.title}')
        tabs['grok']['ready'] = True
        log(f'Grok ready: {d.title}')

    el = d.find_element(By.CSS_SELECTOR, 'textarea, [contenteditable="true"], [role="textbox"]')
    el.send_keys(message); time.sleep(0.3)
    el.send_keys(Keys.RETURN)
    log('Grok: message sent')

    if not wait: return 'Message sent. Use ai_status to check.'

    for i in range(60):
        time.sleep(1)
        if i > 5:
            resp = d.execute_script('''
                const b=document.querySelectorAll('div[class*="response"],div[class*="message-content"],article');
                return b.length>0?b[b.length-1].innerText:null;
            ''')
            if resp and len(resp) > 5: return save_response('grok', resp)
    return 'Error: No response'


def save_response(platform, text):
    ts = time.strftime('%Y%m%d-%H%M%S')
    path = RESPONSE_DIR / f'{platform}-{ts}.md'
    path.write_text(f'# {platform.upper()} — {ts}\n\n{text}')
    if len(text) <= 500: return text
    preview = text[:500].rsplit(' ', 1)[0]
    return f'{preview}...\n\n[Full: {len(text)} chars → {path}]'


# ── Action dispatch ──

def dispatch_action(action, args):
    actions = {
        'search': action_search,
        'navigate': action_navigate,
        'read': action_read,
        'find': action_find,
        'click': action_click,
        'type': action_type,
        'fill_form': action_fill_form,
        'submit': action_submit,
        'screenshot': action_screenshot,
        'scroll': action_scroll,
        'extract_data': action_extract_data,
        'wait_for': action_wait_for,
        'tabs': action_tabs,
        'pipeline': action_pipeline,
    }
    if action in actions:
        return actions[action](args)
    elif action == 'open':
        return action_navigate(args)
    return {'error': f'Unknown action: {action}'}


# ── Cleanup ──

def cleanup():
    global driver
    if driver:
        try: driver.quit()
        except: pass
    for pid in our_pids:
        try: os.kill(pid, 9)
        except: pass
    try: PID_FILE.unlink(missing_ok=True)
    except: pass
    log('Cleanup done')

atexit.register(cleanup)
signal.signal(signal.SIGTERM, lambda *a: (cleanup(), sys.exit(0)))


# ── MCP Protocol ──

TOOLS = [
    {
        "name": "ghost",
        "description": "Browser action. ONE persistent Chrome, multiple tabs. Actions: search, navigate, read, find, click, type, fill_form, submit, screenshot, scroll, extract_data, wait_for, tabs, pipeline.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["search","navigate","read","find","click","type","fill_form","submit","screenshot","scroll","extract_data","wait_for","tabs","pipeline","open"]},
                "url": {"type": "string"}, "query": {"type": "string"}, "text": {"type": "string"},
                "selector": {"type": "string"}, "value": {"type": "string"}, "fields": {"type": "string"},
                "engine": {"type": "string", "enum": ["google","duckduckgo"]},
                "num": {"type": "integer", "default": 10},
                "direction": {"type": "string", "enum": ["up","down"]},
                "amount": {"type": "integer"}, "type_": {"type": "string", "enum": ["table","list","links","product"]},
                "tab": {"type": "string"}, "name": {"type": "string"},
                "by": {"type": "string", "enum": ["text","css","xpath","role"]},
                "index": {"type": "integer"}, "steps": {"type": "string"},
                "wait": {"type": "integer", "default": 5000},
                "profile": {"type": "string"},
            },
            "required": ["action"]
        }
    },
    {
        "name": "gpt",
        "description": "Send message to ChatGPT. Persistent conversation in dedicated tab.",
        "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}, "raw": {"type": "boolean", "default": False}, "wait": {"type": "boolean", "default": True}}, "required": ["message"]}
    },
    {
        "name": "grok",
        "description": "Send message to Grok. Persistent conversation in dedicated tab.",
        "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}, "wait": {"type": "boolean", "default": True}}, "required": ["message"]}
    },
    {
        "name": "ai_status",
        "description": "Check browser and chat status. Shows tabs, active sessions.",
        "inputSchema": {"type": "object", "properties": {}}
    }
]


def respond(id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "result": result}) + '\n')
    sys.stdout.flush()

def respond_error(id, code, msg):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}) + '\n')
    sys.stdout.flush()


def handle(req):
    method = req.get('method', '')
    params = req.get('params', {})
    id = req.get('id')

    if method == 'initialize':
        respond(id, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "neo-browser", "version": "4.0.0"}})
    elif method == 'tools/list':
        respond(id, {"tools": TOOLS})
    elif method == 'tools/call':
        name = params.get('name', '')
        args = params.get('arguments', {})

        if name == 'ghost':
            try:
                result = dispatch_action(args.get('action', 'open'), args)
                text = json.dumps(result, indent=2, ensure_ascii=False) if not isinstance(result, str) else result
                respond(id, {"content": [{"type": "text", "text": text}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'gpt':
            try:
                resp = action_chat_gpt(args.get('message', ''), args.get('wait', True))
                respond(id, {"content": [{"type": "text", "text": resp}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'grok':
            try:
                resp = action_chat_grok(args.get('message', ''), args.get('wait', True))
                respond(id, {"content": [{"type": "text", "text": resp}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'ai_status':
            respond(id, {"content": [{"type": "text", "text": json.dumps({
                'tabs': tab_list(),
                'chrome_alive': driver is not None,
                'pids': list(our_pids),
            }, indent=2)}]})

        else:
            respond_error(id, -32601, f"Unknown tool: {name}")

    elif method == 'notifications/initialized':
        pass
    elif id is not None:
        respond_error(id, -32601, f"Unknown method: {method}")


# ── Main ──
log('MCP server started — persistent Chrome, neomode, multi-tab')

for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        handle(json.loads(line))
    except json.JSONDecodeError:
        log(f'JSON error: {line[:80]}')
    except Exception as e:
        log(f'Error: {e}')
