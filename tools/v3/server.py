#!/usr/bin/env python3
"""
NeoV3 — Unified AI Browser. One MCP, all engines.

FAST PATH (V1 neobrowser CLI): browse, search, read — 500ms-3s
CHROME PATH (undetected-chromedriver neomode): everything else — 5-15s
CHAT (persistent Chrome tabs): gpt, grok

Chrome is lazy-launched on first need. Stays alive between calls.
"""

import json, sys, os, time, subprocess, threading, atexit, signal
from pathlib import Path

def log(msg):
    print(f'[v3] {msg}', file=sys.stderr, flush=True)

# ── Config ──
CHROME_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
PROFILE = os.environ.get('NEOBROWSER_PROFILE', 'Profile 24')
NEOMODE_JS = '''
Object.defineProperty(screen,'width',{get:()=>1920});
Object.defineProperty(screen,'height',{get:()=>1080});
Object.defineProperty(screen,'availWidth',{get:()=>1920});
Object.defineProperty(screen,'availHeight',{get:()=>1055});
Object.defineProperty(window,'outerHeight',{get:()=>1055});
Object.defineProperty(window,'innerHeight',{get:()=>968});
'''
PID_FILE = Path.home() / '.neorender' / 'v3-pids.json'
RESPONSE_DIR = Path.home() / '.neorender' / 'ai-chat-responses'
RESPONSE_DIR.mkdir(parents=True, exist_ok=True)

# ── State ──
_chrome = None
_chrome_lock = threading.Lock()
_chrome_pids = set()
_chrome_tabs = {}  # name → window handle

# Kill stale pids from previous crash
try:
    if PID_FILE.exists():
        for pid in json.loads(PID_FILE.read_text()):
            try: os.kill(int(pid), 9)
            except: pass
        PID_FILE.unlink(missing_ok=True)
        time.sleep(1)
except: pass

# ── V1 Fast Path ──

# V2 binary for fast HTTP path (wreq Chrome TLS + html5ever + WOM + tracing)
V2_BIN = str(Path(__file__).parent.parent.parent / 'target' / 'release' / 'neorender')
# Fallback to V1 if V2 binary not found
V1_BIN = 'neobrowser'

def fast(cmd, url, extra=None, timeout=30):
    """Fast path: V2 binary (Rust HTTP + WOM) or V1 fallback."""
    bin_path = V2_BIN if Path(V2_BIN).exists() else V1_BIN
    args = [bin_path, cmd, url] + (extra or [])
    start = time.time()
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        return r.stdout.strip(), int((time.time()-start)*1000)
    except: return '', 30000

# ── Chrome Neomode (lazy singleton) ──

def chrome():
    global _chrome
    if _chrome:
        try: _ = _chrome.title; return _chrome
        except:
            log('Chrome died, recreating')
            _chrome = None; _chrome_tabs.clear()

    with _chrome_lock:
        if _chrome: return _chrome
        import undetected_chromedriver as uc
        log('Launching Chrome neomode...')
        options = uc.ChromeOptions()
        options.add_argument('--window-size=1920,1080')
        options.add_argument('--no-sandbox')
        options.add_argument('--disable-dev-shm-usage')
        options.add_argument(f'--user-agent={CHROME_UA}')
        options.headless = True

        _chrome = uc.Chrome(options=options, version_main=146)
        if hasattr(_chrome, 'service') and hasattr(_chrome.service, 'process'):
            _chrome_pids.add(_chrome.service.process.pid)
        if hasattr(_chrome, 'browser_pid'):
            _chrome_pids.add(_chrome.browser_pid)
        PID_FILE.parent.mkdir(parents=True, exist_ok=True)
        PID_FILE.write_text(json.dumps(list(_chrome_pids)))

        _chrome.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})
        _chrome_tabs['main'] = _chrome.current_window_handle
        log(f'Chrome ready (pids={_chrome_pids})')
    return _chrome

def chrome_go(url, wait_s=5):
    d = chrome(); d.get(url); time.sleep(wait_s); return d

def chrome_eval(js):
    return chrome().execute_script(js)

def chrome_import_cookies(domain):
    d = chrome()
    script = Path(__file__).parent.parent / 'spa-clone' / 'import-cookies.mjs'
    if not script.exists(): return
    try:
        r = subprocess.run(['node', str(script), domain, '--profile', PROFILE, '--json'],
                           capture_output=True, text=True, timeout=15)
        if r.returncode != 0: return
        cookies = json.loads(r.stdout)
        d.get(f'https://{domain}'); time.sleep(1)
        ok = 0
        for c in cookies:
            try:
                ck = {'name': c['name'], 'value': c['value']}
                if c.get('domain'): ck['domain'] = c['domain']
                if c.get('path'): ck['path'] = c['path']
                if c.get('secure'): ck['secure'] = c['secure']
                if c.get('http_only'): ck['httpOnly'] = c['http_only']
                d.add_cookie(ck); ok += 1
            except: pass
        log(f'Cookies: {ok}/{len(cookies)} for {domain}')
    except Exception as e:
        log(f'Cookie import failed: {e}')

def save_response(text, platform):
    if len(text) <= 500: return text
    ts = time.strftime('%Y%m%d-%H%M%S')
    p = RESPONSE_DIR / f'{platform}-{ts}.md'
    p.write_text(text)
    return text[:500] + f'...\n[Full: {len(text)} chars → {p}]'

# ── Actions ──

def act_browse(a):
    url = a.get('url','')
    if not url: return 'url required'
    # Try V2/V1 fast path first
    out, ms = fast('see', url)
    if len(out) > 200:
        # V2 returns JSON — extract compact view
        try:
            data = json.loads(out)
            title = data.get('title', '')
            nodes = data.get('wom', {}).get('nodes', [])
            if nodes:
                lines = [f'{title} | {url}\n']
                for n in nodes[:50]:
                    label = n.get('label', '')
                    tag = n.get('tag', '')
                    href = n.get('href', '')
                    if label:
                        if href:
                            lines.append(f'  [{tag}] {label} → {href}')
                        else:
                            lines.append(f'  [{tag}] {label}')
                log(f'V2 browse: {ms}ms, {len(nodes)} nodes')
                return '\n'.join(lines)
        except json.JSONDecodeError:
            pass
        # V1 returns plain text
        log(f'V1 browse: {ms}ms')
        return out
    # Chrome fallback for empty results (SPAs, CF)
    log(f'Fast path empty, Chrome fallback...')
    d = chrome_go(url, int(a.get('wait', 8000))/1000)
    info = d.execute_script('return {title:document.title,text:document.body?.innerText?.substring(0,3000)||"",elements:document.querySelectorAll("*").length}')
    return f'{info["title"]} | {url}\n{info["elements"]} elements\n\n{info["text"]}'

def act_search(a):
    q = a.get('query','')
    if not q: return 'query required'
    out = fast('search', q, ['--num', str(a.get('num',10))])[0]
    return out or 'No results'

def act_read(a):
    url = a.get('url','')
    if not url: return 'url required'
    out, ms = fast('see', url)
    if len(out) > 100:
        log(f'V1 read: {ms}ms')
        return '\n'.join(l for l in out.split('\n') if l.strip() and not l.startswith('==='))[:3000]
    d = chrome_go(url, 3)
    return d.execute_script('''
        const s=['main','article','[role="main"]','#content'];
        let el; for(const q of s){el=document.querySelector(q);if(el)break}
        if(!el)el=document.body;
        const c=el.cloneNode(true);
        ['script','style','nav','footer','header'].forEach(t=>c.querySelectorAll(t).forEach(n=>n.remove()));
        return c.innerText.trim().substring(0,3000);
    ''') or ''

def act_navigate(a):
    url = a.get('url','')
    if not url: return 'url required'
    d = chrome_go(url, int(a.get('wait',5000))/1000)
    return json.dumps({'title':d.title,'url':d.current_url,'elements':d.execute_script('return document.querySelectorAll("*").length')})

def act_find(a):
    text = a.get('text', a.get('selector',''))
    by = a.get('by', 'text')
    if not text: return 'text or selector required'
    d = chrome()
    return d.execute_script(f'''
        const q={json.dumps(text)}, by={json.dumps(by)};
        let els=[];
        if(by==='css') els=Array.from(document.querySelectorAll(q));
        else if(by==='xpath'){{const r=document.evaluate(q,document,null,5,null);let n;while(n=r.iterateNext())els.push(n)}}
        else{{const ql=q.toLowerCase();els=Array.from(document.querySelectorAll('*')).filter(e=>
            (e.innerText||'').toLowerCase().includes(ql)||(e.getAttribute('aria-label')||'').toLowerCase().includes(ql)||
            (e.placeholder||'').toLowerCase().includes(ql))}}
        return JSON.stringify(els.slice(0,5).map((e,i)=>({{
            index:i,tag:e.tagName.toLowerCase(),text:(e.innerText||'').substring(0,50),
            selector:e.id?'#'+e.id:e.tagName.toLowerCase()+(e.className?'.'+e.className.split(' ')[0]:''),
            clickable:e.tagName==='A'||e.tagName==='BUTTON'||e.onclick!==null||e.getAttribute('role')==='button',
            type:e.type||null,rect:e.getBoundingClientRect()
        }})));
    ''')

def act_click(a):
    text = a.get('text', a.get('selector',''))
    idx = int(a.get('index', 0))
    if not text: return 'text or selector required'
    d = chrome()
    clicked = d.execute_script(f'''
        const q={json.dumps(text)};
        let els=document.querySelectorAll(q);
        if(!els.length){{const ql=q.toLowerCase();els=Array.from(document.querySelectorAll('a,button,[role=button]'))
            .filter(e=>(e.innerText||'').toLowerCase().includes(ql))}}
        if(els[{idx}]){{els[{idx}].click();return true}}return false;
    ''')
    time.sleep(2)
    return json.dumps({'clicked':bool(clicked),'url':d.current_url,'title':d.title})

def act_type(a):
    sel = a.get('selector', a.get('text',''))
    val = a.get('value','')
    if not sel or not val: return 'selector and value required'
    d = chrome()
    from selenium.webdriver.common.by import By
    try: el = d.find_element(By.CSS_SELECTOR, sel)
    except:
        el = d.execute_script(f'''
            const q={json.dumps(sel)}.toLowerCase();
            return document.querySelector('[placeholder*="'+q+'"]')||document.querySelector('[name*="'+q+'"]')||
                   document.querySelector('[aria-label*="'+q+'"]');
        ''')
    if not el: return f'Not found: {sel}'
    el.clear(); el.send_keys(val)
    return json.dumps({'typed':True,'value':val})

def act_fill_form(a):
    fields = a.get('fields','{}')
    if isinstance(fields, str): fields = json.loads(fields)
    url = a.get('url','')
    if url: chrome_go(url, 5)
    return chrome_eval(f'''
        const f={json.dumps(fields)};const filled=[],errors=[];
        for(const[k,v]of Object.entries(f)){{
            const el=document.querySelector('[name="'+k+'"]')||document.querySelector('#'+k)||
                     document.querySelector('[placeholder*="'+k+'" i]');
            if(el){{
                const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;
                if(s)s.call(el,v);else el.value=v;
                el.dispatchEvent(new Event('input',{{bubbles:true}}));
                el.dispatchEvent(new Event('change',{{bubbles:true}}));
                filled.push(k);
            }}else errors.push(k);
        }}
        return JSON.stringify({{filled,errors}});
    ''')

def act_submit(a):
    sel = a.get('selector','')
    d = chrome()
    return d.execute_script(f'''
        const q={json.dumps(sel)};
        let btn=q?document.querySelector(q):document.querySelector('[type=submit],button[type=submit]');
        if(!btn)btn=document.querySelector('form')?.querySelector('button');
        if(btn){{btn.click();return JSON.stringify({{submitted:true}})}}
        const form=document.querySelector('form');
        if(form){{form.submit();return JSON.stringify({{submitted:true,method:'form.submit'}})}}
        return JSON.stringify({{submitted:false,error:'no submit button or form found'}});
    ''')

def act_screenshot(a):
    url = a.get('url','')
    d = chrome()
    if url: d.get(url); time.sleep(3)
    p = '/tmp/v3-screenshot.png'
    d.save_screenshot(p)
    return json.dumps({'path':p})

def act_scroll(a):
    direction = a.get('direction','down')
    amount = int(a.get('amount', 500))
    dy = amount if direction == 'down' else -amount
    return chrome_eval(f'''
        window.scrollBy(0,{dy});
        return JSON.stringify({{scrolled:true,scroll_y:window.scrollY,page_height:document.body.scrollHeight,
            at_bottom:window.scrollY+window.innerHeight>=document.body.scrollHeight-10}});
    ''')

def act_html(a):
    url = a.get('url','')
    if url: chrome_go(url, 3)
    html = chrome_eval('return document.documentElement.outerHTML')
    if len(html) > 10000:
        return html[:10000] + f'...\n[truncated, {len(html)} total bytes]'
    return html

def act_wait_for(a):
    sel = a.get('selector', a.get('text',''))
    if not sel: return 'selector or text required'
    timeout = int(a.get('wait', 10000)) / 1000
    d = chrome()
    start = time.time()
    while time.time() - start < timeout:
        found = d.execute_script(f'''
            const q={json.dumps(sel)};
            if(document.querySelector(q)) return true;
            return Array.from(document.querySelectorAll('*')).some(e=>(e.innerText||'').includes(q));
        ''')
        if found:
            return json.dumps({'found':True,'elapsed_ms':int((time.time()-start)*1000)})
        time.sleep(0.5)
    return json.dumps({'found':False,'elapsed_ms':int((time.time()-start)*1000)})

def act_login(a):
    url = a.get('url','')
    email = a.get('email','')
    password = a.get('password','')
    if not url or not email or not password: return 'url, email, password required'
    d = chrome_go(url, 5)
    d.execute_script(f'''
        const e={json.dumps(email)},p={json.dumps(password)};
        const ei=document.querySelector('[type=email],[name=email],[name=username],[autocomplete=email]');
        const pi=document.querySelector('[type=password]');
        if(ei){{const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;
            if(s)s.call(ei,e);else ei.value=e;ei.dispatchEvent(new Event('input',{{bubbles:true}}))}}
        if(pi){{const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;
            if(s)s.call(pi,p);else pi.value=p;pi.dispatchEvent(new Event('input',{{bubbles:true}}))}}
    ''')
    time.sleep(1)
    d.execute_script('document.querySelector("[type=submit],button[type=submit]")?.click()')
    time.sleep(5)
    return json.dumps({'title':d.title,'url':d.current_url,'logged_in':'login' not in d.current_url.lower()})

def act_extract_data(a):
    type_ = a.get('type_', a.get('type', 'table'))
    d = chrome()
    if type_ == 'table':
        return d.execute_script('''
            const t=document.querySelector('table');if(!t)return '[]';
            const rows=Array.from(t.querySelectorAll('tr'));
            return JSON.stringify(rows.map(r=>Array.from(r.querySelectorAll('th,td')).map(c=>c.innerText.trim())));
        ''')
    elif type_ == 'links':
        return d.execute_script('''
            return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,50).map(a=>({text:a.innerText.trim().substring(0,60),href:a.href})));
        ''')
    return '[]'

# ── Chat (persistent tabs) ──

def chat_gpt(msg, wait=True):
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys
    from selenium.webdriver.support.ui import WebDriverWait
    from selenium.webdriver.support import expected_conditions as EC

    d = chrome()
    if 'chatgpt' not in _chrome_tabs:
        d.switch_to.new_window('tab')
        _chrome_tabs['chatgpt'] = d.current_window_handle
        chrome_import_cookies('chatgpt.com')
        chrome_import_cookies('openai.com')
        d.get('https://chatgpt.com'); time.sleep(8)
        log(f'ChatGPT tab: {d.title}')
    else:
        d.switch_to.window(_chrome_tabs['chatgpt'])

    el = WebDriverWait(d, 10).until(EC.presence_of_element_located((By.ID, 'prompt-textarea')))
    el.click(); time.sleep(0.3); el.send_keys(msg); time.sleep(0.5)
    try: d.find_element(By.CSS_SELECTOR, '[data-testid="send-button"]').click()
    except: el.send_keys(Keys.RETURN)
    log('GPT: sent')
    if not wait: return 'Sent.'

    for i in range(120):
        time.sleep(1)
        streaming = d.execute_script('return !!document.querySelector("[data-testid=stop-button]")')
        if not streaming and i > 3:
            resp = d.execute_script('const m=document.querySelectorAll("[data-message-author-role=assistant]");return m.length?m[m.length-1].innerText:null')
            if resp: return save_response(resp, 'gpt')
    return 'No response after 120s'

def chat_grok(msg, wait=True):
    from selenium.webdriver.common.keys import Keys
    d = chrome()
    if 'grok' not in _chrome_tabs:
        d.switch_to.new_window('tab')
        _chrome_tabs['grok'] = d.current_window_handle
        d.get('https://grok.com'); time.sleep(8)
        log(f'Grok tab: {d.title}')
    else:
        d.switch_to.window(_chrome_tabs['grok'])

    el = d.find_element('css selector', 'textarea, [contenteditable="true"], [role="textbox"]')
    el.send_keys(msg); time.sleep(0.3); el.send_keys(Keys.RETURN)
    log('Grok: sent')
    if not wait: return 'Sent.'

    for i in range(60):
        time.sleep(1)
        if i > 5:
            resp = d.execute_script('const b=document.querySelectorAll("div[class*=response],div[class*=message-content],article");return b.length>0?b[b.length-1].innerText:null')
            if resp and len(resp) > 5: return save_response(resp, 'grok')
    return 'No response after 60s'

# ── Dispatch ──

ACTIONS = {
    'browse': act_browse, 'search': act_search, 'read': act_read,
    'navigate': act_navigate, 'open': act_browse,
    'find': act_find, 'click': act_click, 'type': act_type,
    'fill_form': act_fill_form, 'submit': act_submit,
    'screenshot': act_screenshot, 'scroll': act_scroll, 'html': act_html,
    'wait_for': act_wait_for, 'login': act_login, 'extract_data': act_extract_data,
}

# ── Cleanup ──

def cleanup():
    if _chrome:
        try: _chrome.quit()
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
    {"name":"ghost","description":"V3 AI browser. FAST: browse/search/read (V1 HTTP ~1s). CHROME: click/type/fill_form/navigate/find/scroll/screenshot/login/submit/wait_for/extract_data/html (neomode ~5s). CF bypass. Use browse for reading, search for searching, navigate+click+type for interaction.",
     "inputSchema":{"type":"object","properties":{
        "action":{"type":"string","enum":list(ACTIONS.keys())},
        "url":{"type":"string"},"query":{"type":"string"},"text":{"type":"string"},
        "selector":{"type":"string"},"value":{"type":"string"},"fields":{"type":"string"},
        "by":{"type":"string","enum":["text","css","xpath","role"]},
        "direction":{"type":"string","enum":["up","down"]},
        "amount":{"type":"integer"},"index":{"type":"integer"},
        "num":{"type":"integer","default":10},"wait":{"type":"integer","default":5000},
        "email":{"type":"string"},"password":{"type":"string"},
        "type_":{"type":"string","enum":["table","links"]},
     },"required":["action"]}},
    {"name":"gpt","description":"ChatGPT. Persistent conversation.","inputSchema":{"type":"object","properties":{"message":{"type":"string"},"raw":{"type":"boolean","default":False},"wait":{"type":"boolean","default":True}},"required":["message"]}},
    {"name":"grok","description":"Grok. Persistent conversation.","inputSchema":{"type":"object","properties":{"message":{"type":"string"},"wait":{"type":"boolean","default":True}},"required":["message"]}},
    {"name":"ai_status","description":"Status of Chrome and chat sessions.","inputSchema":{"type":"object","properties":{}}},
]

def respond(id, result):
    sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":id,"result":result})+'\n'); sys.stdout.flush()

def respond_err(id, code, msg):
    sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})+'\n'); sys.stdout.flush()

def handle(req):
    method, params, id = req.get('method',''), req.get('params',{}), req.get('id')
    if method == 'initialize':
        respond(id, {"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"neo-v3","version":"3.0.0"}})
    elif method == 'tools/list':
        respond(id, {"tools":TOOLS})
    elif method == 'tools/call':
        name, args = params.get('name',''), params.get('arguments',{})
        try:
            if name == 'ghost':
                action = args.get('action','browse')
                fn = ACTIONS.get(action)
                if fn:
                    result = fn(args)
                    text = result if isinstance(result, str) else json.dumps(result, ensure_ascii=False)
                    respond(id, {"content":[{"type":"text","text":text}]})
                else:
                    respond_err(id, -32602, f'Unknown action: {action}')
            elif name == 'gpt':
                resp = chat_gpt(args['message'], args.get('wait', True))
                respond(id, {"content":[{"type":"text","text":resp}]})
            elif name == 'grok':
                resp = chat_grok(args['message'], args.get('wait', True))
                respond(id, {"content":[{"type":"text","text":resp}]})
            elif name == 'ai_status':
                respond(id, {"content":[{"type":"text","text":json.dumps({"chrome":_chrome is not None,"tabs":list(_chrome_tabs.keys()),"pids":list(_chrome_pids)})}]})
            else:
                respond_err(id, -32601, f'Unknown: {name}')
        except Exception as e:
            respond(id, {"content":[{"type":"text","text":f"Error: {e}"}],"isError":True})
    elif method == 'notifications/initialized': pass
    elif id is not None: respond_err(id, -32601, f'Unknown: {method}')

log('V3 started — V1 fast + Chrome neomode + GPT/Grok chat')
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: handle(json.loads(line))
    except json.JSONDecodeError: log(f'JSON err: {line[:80]}')
    except Exception as e: log(f'Error: {e}')
