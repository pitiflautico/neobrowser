#!/usr/bin/env python3
"""
NeoBrowser V3 — AI Browser MCP Server.

Tools:
  BROWSE  — Fast HTTP browse (V1, ~1s). Best for reading pages.
  SEARCH  — Web search via DuckDuckGo (~1s).
  OPEN    — Open URL in Chrome ghost (headless, CF bypass, ~5s).
  READ    — Extract clean text from current page.
  FIND    — Find element by text, CSS, XPath, or role.
  CLICK   — Click element by text or CSS selector.
  TYPE    — Type text in input field.
  FILL    — Fill form fields.
  SUBMIT  — Submit current form.
  SCROLL  — Scroll page up/down.
  SCREENSHOT — Capture page screenshot.
  WAIT    — Wait for element or text to appear.
  LOGIN   — Fill email+password and submit.
  EXTRACT — Extract structured data (tables, links).
  GPT     — Send/read ChatGPT. Actions: send, read_last, is_streaming, history.
  GROK    — Send/read Grok. Actions: send, read_last, is_streaming, history.
  PLUGIN  — Run/list/create reusable browser pipelines (~/.neorender/plugins/).
  STATUS  — Browser and chat session status.
"""

import json, sys, os, time, subprocess, threading, atexit, signal, tempfile, re, urllib.request, urllib.parse
from pathlib import Path

def log(msg):
    print(f'[neo] {msg}', file=sys.stderr, flush=True)

# ── Config ──
CHROME_BIN = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
CHROME_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
PROFILE = os.environ.get('NEOBROWSER_PROFILE', 'Profile 24')
V1_BIN = 'neobrowser'
RESPONSE_DIR = Path.home() / '.neorender' / 'responses'
RESPONSE_DIR.mkdir(parents=True, exist_ok=True)
PID_FILE = Path.home() / '.neorender' / 'neo-browser-pids.json'

NEOMODE_JS = '''
Object.defineProperty(screen,'width',{get:()=>1920});
Object.defineProperty(screen,'height',{get:()=>1080});
Object.defineProperty(screen,'availWidth',{get:()=>1920});
Object.defineProperty(screen,'availHeight',{get:()=>1055});
Object.defineProperty(window,'outerHeight',{get:()=>1055});
Object.defineProperty(window,'innerHeight',{get:()=>968});
'''

SANITIZE_JS = '''(function(){
    const r={title:document.title,url:location.href};
    const u=location.href.toLowerCase();
    if(u.includes('login')||u.includes('sign_in'))r.type='login';
    else if(u.includes('search')||u.includes('query'))r.type='search';
    else if(u.includes('messaging')||u.includes('messages'))r.type='messaging';
    else if(document.querySelectorAll('article').length>1)r.type='feed';
    else if(document.querySelector('article'))r.type='article';
    else r.type='page';
    r.auth=!!document.querySelector('[class*="avatar"],[class*="profile-photo"]')?'logged-in':'anonymous';
    r.headings=Array.from(document.querySelectorAll('h1,h2,h3')).slice(0,10).map(h=>h.innerText.trim()).filter(t=>t.length>0&&t.length<200);
    const main=document.querySelector('main,article,[role="main"],#content,.content')||document.body;
    const clone=main.cloneNode(true);
    ['script','style','nav','footer','header','aside','svg','noscript'].forEach(t=>clone.querySelectorAll(t).forEach(n=>n.remove()));
    r.text=clone.innerText.trim().replace(/\\n{3,}/g,'\\n\\n').substring(0,3000);
    r.forms=Array.from(document.querySelectorAll('form')).slice(0,5).map(f=>{
        const fields=Array.from(f.querySelectorAll('input:not([type=hidden]),textarea,select')).map(i=>({
            type:i.type||i.tagName.toLowerCase(),name:i.name||i.id||'',
            placeholder:i.placeholder||'',label:(i.labels?.[0]?.innerText||i.getAttribute('aria-label')||'').trim()
        }));
        const sub=f.querySelector('[type=submit],button[type=submit],button:not([type])');
        return{fields,submit:sub?.innerText?.trim()||''};
    });
    const seen=new Set();
    r.links=Array.from(document.querySelectorAll('a[href]')).filter(a=>{
        const t=a.innerText.trim(),h=a.href;
        if(!t||t.length>100||t.length<2||h.startsWith('javascript:')||seen.has(t))return false;
        seen.add(t);return true;
    }).slice(0,15).map(a=>({text:a.innerText.trim(),href:a.href}));
    r.buttons=Array.from(document.querySelectorAll('button,[role="button"]')).map(b=>b.innerText.trim()).filter(t=>t.length>0&&t.length<50).slice(0,10);
    return JSON.stringify(r);
})()'''

# ── State ──
_chrome = None
_chrome_lock = threading.Lock()
_chrome_pids = set()
_chrome_tabs = {}

# Kill stale pids
try:
    if PID_FILE.exists():
        for pid in json.loads(PID_FILE.read_text()):
            try: os.kill(int(pid), 9)
            except: pass
        PID_FILE.unlink(missing_ok=True)
        time.sleep(1)
except: pass

# ── V1 Fast Path ──

def fast(cmd, url, extra=None, timeout=30):
    args = [V1_BIN, cmd, url] + (extra or [])
    start = time.time()
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        return r.stdout.strip(), int((time.time()-start)*1000)
    except: return '', 30000

# ── Ghost Chrome (headless CDP, no chromedriver) ──

try:
    import websockets.sync.client as ws_sync
except ImportError:
    log('ERROR: pip install websockets')
    sys.exit(1)

class GhostChrome:
    def __init__(self, proc, port, ws, profile_dir):
        self.proc = proc
        self.port = port
        self.ws = ws
        self.profile_dir = profile_dir
        self._id = 10

    def _send(self, method, params=None):
        self._id += 1
        self.ws.send(json.dumps({'id': self._id, 'method': method, 'params': params or {}}))
        while True:
            data = json.loads(self.ws.recv(timeout=30))
            if data.get('id') == self._id:
                return data.get('result', {})

    def js(self, code):
        expr = f'(function(){{{code}}})()' if 'return ' in code else code
        r = self._send('Runtime.evaluate', {'expression': expr, 'returnByValue': True, 'awaitPromise': False})
        return r.get('result', {}).get('value')

    def go(self, url):
        self._send('Page.navigate', {'url': url})

    def screenshot(self, path):
        import base64
        r = self._send('Page.captureScreenshot', {'format': 'png'})
        with open(path, 'wb') as f: f.write(base64.b64decode(r.get('data', '')))

    def cookie(self, name, value, domain, path='/', secure=False, httpOnly=False, expires=None):
        p = {'name': name, 'value': value, 'domain': domain, 'path': path,
             'secure': secure, 'httpOnly': httpOnly, 'url': f"https://{domain.lstrip('.')}"}
        if expires: p['expires'] = expires
        self._send('Network.setCookie', p)

    def key(self, text):
        for c in text:
            self._send('Input.dispatchKeyEvent', {'type': 'keyDown', 'text': c, 'key': c, 'windowsVirtualKeyCode': ord(c)})
            self._send('Input.dispatchKeyEvent', {'type': 'keyUp', 'key': c})

    def enter(self):
        self._send('Input.dispatchKeyEvent', {'type': 'keyDown', 'key': 'Enter', 'code': 'Enter', 'windowsVirtualKeyCode': 13, 'text': '\r'})
        self._send('Input.dispatchKeyEvent', {'type': 'keyUp', 'key': 'Enter', 'code': 'Enter'})

    @property
    def title(self): return self.js('document.title') or ''

    @property
    def url(self): return self.js('location.href') or ''

    def quit(self):
        try: self.ws.close()
        except: pass
        try: self.proc.kill()
        except: pass

    def sanitize(self):
        try:
            data = json.loads(self.js(SANITIZE_JS))
            lines = [f'[{data.get("type","page")}] ({data.get("auth","")}) {data["title"]}', f'url: {data["url"]}', '']
            for h in data.get('headings', [])[:5]: lines.append(f'# {h}')
            for f in data.get('forms', []):
                if f['fields']:
                    lines.append('')
                    for fd in f['fields']:
                        lbl = fd.get('label') or fd.get('placeholder') or fd.get('name') or fd['type']
                        lines.append(f'  [{fd["type"]}] {lbl}')
                    if f.get('submit'): lines.append(f'  [submit] {f["submit"]}')
            btns = data.get('buttons', [])
            if btns: lines.extend(['', f'[btn] {" | ".join(btns[:10])}'])
            links = data.get('links', [])
            if links:
                lines.extend(['', f'[links] {len(links)}'])
                for l in links[:15]: lines.append(f'  {l["text"]} → {l["href"]}')
            text = data.get('text', '')
            if text: lines.extend(['', text[:1500] + ('...' if len(text) > 1500 else '')])
            return '\n'.join(lines)
        except Exception as e:
            return f'{self.title}\n\n{self.js("return document.body?.innerText?.substring(0,2000)") or ""}'


def _kill_pids():
    for pid in list(_chrome_pids):
        try: os.kill(pid, 9)
        except: _chrome_pids.discard(pid)

def chrome():
    global _chrome
    if _chrome:
        try:
            # Verify websocket is alive with a real JS eval, not just property access
            result = _chrome.js('return document.readyState')
            if result: return _chrome
            raise Exception('dead ws')
        except:
            try: _chrome.quit()
            except: pass
            _kill_pids()
            _chrome = None; _chrome_tabs.clear(); _chrome_pids.clear(); time.sleep(1)

    with _chrome_lock:
        if _chrome: return _chrome
        for attempt in range(3):
            try:
                if attempt > 0: _kill_pids(); time.sleep(2)
                log('Launching Ghost Chrome...')
                import socket, shutil

                # Persistent profile with synced cookies from real Chrome
                ghost_dir = Path.home() / '.neorender' / 'ghost-profile'
                ghost_default = ghost_dir / 'Default'
                ghost_default.mkdir(parents=True, exist_ok=True)
                # Clean stale locks from crashed sessions
                for lock in ['SingletonLock', 'SingletonSocket', 'SingletonCookie']:
                    (ghost_dir / lock).unlink(missing_ok=True)

                # Sync session data from real Chrome profile
                real_profile = Path.home() / 'Library' / 'Application Support' / 'Google' / 'Chrome' / PROFILE
                if real_profile.exists():
                    _sync_session(real_profile, ghost_default)

                s = socket.socket(); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.close()
                proc = subprocess.Popen([CHROME_BIN, f'--remote-debugging-port={port}',
                    f'--user-data-dir={str(ghost_dir)}', '--headless=new', '--no-first-run',
                    '--disable-background-networking', '--disable-dev-shm-usage',
                    '--window-size=1920,1080', f'--user-agent={CHROME_UA}', 'about:blank'],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                _chrome_pids.add(proc.pid); time.sleep(2)

                # Get page WS URL
                def http(path):
                    s = socket.socket(); s.settimeout(3); s.connect(('127.0.0.1', port))
                    s.send(f'GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n'.encode())
                    d = b''
                    while True:
                        try: c = s.recv(4096)
                        except: break
                        if not c: break
                        d += c
                        if len(d) > 200: break
                    s.close()
                    return d.split(b'\r\n\r\n', 1)[1] if b'\r\n\r\n' in d else d

                targets = json.loads(http('/json/list'))
                ws_url = [t['webSocketDebuggerUrl'] for t in targets if t['type'] == 'page'][0]
                ws = ws_sync.connect(ws_url, max_size=10_000_000)
                _chrome = GhostChrome(proc, port, ws, str(ghost_dir))
                _chrome._send('Page.enable'); _chrome._send('Network.enable')
                _chrome._send('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})
                _chrome._send('Emulation.setDeviceMetricsOverride', {'width': 1920, 'height': 1080, 'deviceScaleFactor': 1, 'mobile': False})

                PID_FILE.parent.mkdir(parents=True, exist_ok=True)
                PID_FILE.write_text(json.dumps(list(_chrome_pids)))
                log(f'Ghost Chrome ready (port={port}, pid={proc.pid}, profile={PROFILE})')
                return _chrome
            except Exception as e:
                log(f'Chrome attempt {attempt+1} failed: {e}'); _chrome = None
        raise RuntimeError('Chrome failed after 3 attempts')


def _sync_session(src_profile, dst_profile):
    """Sync cookies and session data from real Chrome profile to Ghost profile.
    Both profiles use the same macOS Keychain encryption key, so cookies decrypt fine."""
    import shutil, sqlite3

    # Files to sync: cookies (auth), Local Storage (tokens), Session Storage
    for name in ['Cookies', 'Cookies-journal']:
        src = src_profile / name
        dst = dst_profile / name
        if src.exists():
            try:
                # Copy via SQLite backup to handle WAL mode safely
                if name == 'Cookies' and src.stat().st_size > 0:
                    try:
                        conn_src = sqlite3.connect(f'file:{src}?mode=ro&nolock=1', uri=True)
                        conn_dst = sqlite3.connect(str(dst))
                        conn_src.backup(conn_dst)
                        conn_dst.close(); conn_src.close()
                        log(f'Synced {name} (SQLite backup)')
                        continue
                    except Exception as e:
                        log(f'SQLite backup failed for {name}: {e}, falling back to copy')
                shutil.copy2(str(src), str(dst))
                log(f'Synced {name} (file copy)')
            except Exception as e:
                log(f'Failed to sync {name}: {e}')

    # Sync Local Storage (contains auth tokens for many SPAs)
    for dirname in ['Local Storage', 'Session Storage']:
        src_dir = src_profile / dirname
        dst_dir = dst_profile / dirname
        if src_dir.exists():
            try:
                if dst_dir.exists(): shutil.rmtree(str(dst_dir))
                shutil.copytree(str(src_dir), str(dst_dir))
                log(f'Synced {dirname}/')
            except Exception as e:
                log(f'Failed to sync {dirname}: {e}')

def chrome_go(url, wait_s=5):
    """Navigate Ghost Chrome to URL. Cookies come from profile sync, no export needed."""
    d = chrome()
    _chrome_tabs.clear()  # Single tab — navigating away invalidates chat state
    d.go(url); time.sleep(wait_s)
    return d

def save(text, tag='response'):
    if not text: return 'No content'
    if len(text) <= 500: return text
    ts = time.strftime('%Y%m%d-%H%M%S')
    p = RESPONSE_DIR / f'{tag}-{ts}.md'
    p.write_text(text)
    return text[:500] + f'...\n[Full: {len(text)} chars → {p}]'

# ── Tool implementations ──

def tool_browse(args):
    url = args.get('url', '')
    if not url: return 'url required'
    out, ms = fast('see', url)
    if len(out) > 200:
        log(f'V1 browse: {ms}ms')
        return out
    log('V1 empty, Chrome fallback...')
    d = chrome_go(url)
    return d.sanitize()

def tool_search(args):
    q = args.get('query', '')
    if not q: return 'query required'
    num = int(args.get('num', 10))
    try:
        r = urllib.request.urlopen(urllib.request.Request(
            f'https://html.duckduckgo.com/html/?q={urllib.parse.quote(q)}',
            headers={'User-Agent': 'Mozilla/5.0'}
        ), timeout=10)
        html = r.read().decode()
        results = []
        for m in re.finditer(r'<a rel="nofollow" class="result__a" href="([^"]+)"[^>]*>(.*?)</a>', html):
            raw_url, title = m.group(1), re.sub(r'<[^>]+>', '', m.group(2)).strip()
            # Extract real URL from DDG redirect
            uddg = re.search(r'uddg=([^&]+)', raw_url)
            url = urllib.parse.unquote(uddg.group(1)) if uddg else raw_url
            if url and title:
                results.append(f'{title}\n  {url}')
                if len(results) >= num: break
        return '\n\n'.join(results) if results else 'No results'
    except Exception as e:
        return f'Search error: {e}'

def tool_open(args):
    url = args.get('url', '')
    if not url: return 'url required'
    d = chrome_go(url, int(args.get('wait', 5000)) / 1000)
    return d.sanitize()

SMART_EXTRACTORS = {
    'tweets': '''
        const tweets = document.querySelectorAll('article[data-testid="tweet"], article[role="article"]');
        if (!tweets.length) return 'No tweets found';
        return Array.from(tweets).slice(0, 20).map(t => {
            // Author: extract @handle from profile links
            const links = Array.from(t.querySelectorAll('a[role="link"][href^="/"]'));
            let handle = '';
            for (const a of links) {
                const href = a.getAttribute('href') || '';
                const m = href.match(/^\\/([a-zA-Z0-9_]+)$/);
                if (m && !['home','explore','search','notifications','messages','settings','i'].includes(m[1])) {
                    handle = '@' + m[1];
                    break;
                }
            }
            const name = t.querySelector('[data-testid="User-Name"]')?.innerText?.split('\\n')?.[0] || '';
            const author = handle ? (name ? name + ' ' + handle : handle) : name;
            const text = t.querySelector('[data-testid="tweetText"], [lang]')?.innerText || '';
            const time = t.querySelector('time')?.getAttribute('datetime') || '';
            const stats = Array.from(t.querySelectorAll('[data-testid$="count"], [aria-label*="like"], [aria-label*="repost"]'))
                .map(s => s.getAttribute('aria-label') || s.innerText).filter(Boolean).join(' · ');
            return [author, time, text.substring(0, 500), stats].filter(Boolean).join('\\n');
        }).join('\\n---\\n');
    ''',
    'posts': '''
        const posts = document.querySelectorAll('article, [role="article"], .post, .entry, .feed-item, [class*="post"]');
        if (!posts.length) return 'No posts found';
        return Array.from(posts).slice(0, 15).map(p => {
            const title = p.querySelector('h1,h2,h3,h4,[class*="title"]')?.innerText || '';
            const author = p.querySelector('[class*="author"],[class*="user"],[rel="author"],a[href*="/u/"]')?.innerText || '';
            const text = p.querySelector('[class*="content"],[class*="body"],p')?.innerText || p.innerText;
            const time = p.querySelector('time')?.innerText || p.querySelector('[class*="date"],[class*="time"]')?.innerText || '';
            return [title, author, time, text.substring(0, 500)].filter(Boolean).join('\\n');
        }).join('\\n---\\n');
    ''',
    'comments': '''
        const comments = document.querySelectorAll('[class*="comment"], [data-testid*="comment"], .reply, [class*="Comment"]');
        if (!comments.length) return 'No comments found';
        return Array.from(comments).slice(0, 20).map(c => {
            const author = c.querySelector('[class*="author"],[class*="user"],a[href*="/u/"]')?.innerText || '';
            const text = c.querySelector('[class*="body"],[class*="content"],p')?.innerText || c.innerText;
            const time = c.querySelector('time,[class*="date"]')?.innerText || '';
            return [author, time, text.substring(0, 300)].filter(Boolean).join('\\n');
        }).join('\\n---\\n');
    ''',
    'products': '''
        const items = document.querySelectorAll('[class*="product"],[class*="item"],[data-testid*="product"],[class*="card"]');
        if (!items.length) return 'No products found';
        return Array.from(items).slice(0, 20).map(p => {
            const name = p.querySelector('h2,h3,h4,[class*="title"],[class*="name"]')?.innerText || '';
            const price = p.querySelector('[class*="price"],[data-testid*="price"]')?.innerText || '';
            const link = p.querySelector('a')?.href || '';
            return [name, price, link].filter(Boolean).join(' | ');
        }).join('\\n');
    ''',
    'table': '''
        const tables = document.querySelectorAll('table');
        if (!tables.length) return 'No tables found';
        return Array.from(tables).slice(0, 3).map(t =>
            Array.from(t.querySelectorAll('tr')).slice(0, 50).map(r =>
                Array.from(r.querySelectorAll('th,td')).map(c => c.innerText.trim()).join(' | ')
            ).join('\\n')
        ).join('\\n\\n');
    ''',
}

def tool_read(args):
    url = args.get('url', '')
    selector = args.get('selector', '')
    content_type = args.get('type', '')
    if url:
        if not selector and not content_type:
            out, ms = fast('see', url)
            if len(out) > 100:
                log(f'V1 read: {ms}ms')
                return out[:3000]
        chrome_go(url, 3)
    d = chrome()

    # Smart extractor by content type
    if content_type:
        js = SMART_EXTRACTORS.get(content_type.lower())
        if not js:
            return f'Unknown type: {content_type}. Available: {", ".join(SMART_EXTRACTORS.keys())}'
        text = d.js(js)
        return save(text or f'No {content_type} found on page', 'read')

    # CSS selector extraction
    if selector:
        text = d.js(f'''
            const els = document.querySelectorAll({json.dumps(selector)});
            if (!els.length) return 'No matches for selector';
            return Array.from(els).map(el => el.innerText.trim()).filter(t => t.length > 0).join('\\n---\\n');
        ''')
        return save(text or f'No content for: {selector}', 'read')

    return d.sanitize()

def tool_find(args):
    text = args.get('text', args.get('selector', ''))
    by = args.get('by', 'text')
    if not text: return 'text or selector required'
    return chrome().js(f'''
        const q={json.dumps(text)},by={json.dumps(by)};let els=[];
        if(by==='css')els=Array.from(document.querySelectorAll(q));
        else if(by==='xpath'){{const r=document.evaluate(q,document,null,5,null);let n;while(n=r.iterateNext())els.push(n)}}
        else{{const ql=q.toLowerCase();els=Array.from(document.querySelectorAll('*')).filter(e=>
            (e.innerText||'').toLowerCase().includes(ql)||(e.getAttribute('aria-label')||'').toLowerCase().includes(ql)||
            (e.placeholder||'').toLowerCase().includes(ql))}}
        return JSON.stringify(els.slice(0,5).map((e,i)=>({{
            index:i,tag:e.tagName.toLowerCase(),text:(e.innerText||'').substring(0,80),
            clickable:e.tagName==='A'||e.tagName==='BUTTON'||e.getAttribute('role')==='button'
        }})));
    ''') or '[]'

def tool_click(args):
    text = args.get('text', args.get('selector', ''))
    if not text: return 'text or selector required'
    d = chrome()
    clicked = d.js(f'''
        const q={json.dumps(text)};let els=document.querySelectorAll(q);
        if(!els.length){{const ql=q.toLowerCase();els=Array.from(document.querySelectorAll('a,button,[role=button]'))
            .filter(e=>(e.innerText||'').toLowerCase().includes(ql))}}
        if(els[0]){{els[0].click();return true}}return false;
    ''')
    time.sleep(2)
    return f'Clicked "{text}"\n\n{d.sanitize()}' if clicked else f'Not found: "{text}"'

def tool_type(args):
    """Smart type — finds input by label, placeholder, name, aria-label, or CSS."""
    sel = args.get('selector', args.get('text', ''))
    val = args.get('value', '')
    if not sel or not val: return 'selector and value required'
    d = chrome()
    found = d.js(f'''
        const key = {json.dumps(sel)};
        const kl = key.toLowerCase();
        // Try CSS selector first
        let el = document.querySelector(key);
        // By placeholder
        if (!el) el = document.querySelector('[placeholder*="'+kl+'" i]');
        // By name
        if (!el) el = document.querySelector('[name*="'+kl+'" i]');
        // By aria-label
        if (!el) el = document.querySelector('[aria-label*="'+kl+'" i]');
        // By label text
        if (!el) {{
            const labels = document.querySelectorAll('label');
            for (const lbl of labels) {{
                if (lbl.innerText.toLowerCase().includes(kl)) {{
                    el = lbl.htmlFor ? document.getElementById(lbl.htmlFor) : lbl.querySelector('input,textarea');
                    if (el) break;
                }}
            }}
        }}
        // By type (email, password, etc.)
        if (!el) el = document.querySelector('[type="'+key+'"]');
        if (el) {{
            el.focus();
            el.click();
            // Clear existing value
            if (el.value) el.value = '';
            el.dispatchEvent(new Event('focus', {{bubbles:true}}));
            return JSON.stringify({{found: true, tag: el.tagName, name: el.name||'', placeholder: el.placeholder||''}});
        }}
        return JSON.stringify({{found: false}});
    ''')
    try:
        info = json.loads(found) if found else {'found': False}
    except:
        info = {'found': False}

    if not info.get('found'):
        return f'Not found: "{sel}"'

    d.key(val)
    return json.dumps({'typed': True, 'value': val, 'field': info})

def tool_fill(args):
    """Smart fill — handles inputs, textareas, selects, checkboxes, radios.
    Finds fields by: name, id, placeholder, label text, aria-label, type."""
    fields = args.get('fields', '{}')
    if isinstance(fields, str): fields = json.loads(fields)
    url = args.get('url', '')
    if url: chrome_go(url, 5)
    return chrome().js(f'''
        const fields = {json.dumps(fields)};
        const filled = [], errors = [];

        function findField(key) {{
            const kl = key.toLowerCase();
            // 1. By name/id exact
            let el = document.querySelector('[name="'+key+'"]') || document.querySelector('#'+key);
            if (el) return el;
            // 2. By placeholder (case insensitive)
            el = document.querySelector('[placeholder*="'+kl+'" i]');
            if (el) return el;
            // 3. By aria-label
            el = document.querySelector('[aria-label*="'+kl+'" i]');
            if (el) return el;
            // 4. By label text → linked input
            const labels = document.querySelectorAll('label');
            for (const lbl of labels) {{
                if (lbl.innerText.toLowerCase().includes(kl)) {{
                    if (lbl.htmlFor) return document.getElementById(lbl.htmlFor);
                    const input = lbl.querySelector('input,textarea,select');
                    if (input) return input;
                }}
            }}
            // 5. By type (email, password, tel, etc.)
            el = document.querySelector('[type="'+key+'"]');
            if (el) return el;
            // 6. By visible text near input (label-like spans/divs before input)
            const allInputs = document.querySelectorAll('input,textarea,select');
            for (const inp of allInputs) {{
                const prev = inp.previousElementSibling || inp.parentElement;
                if (prev && prev.innerText && prev.innerText.toLowerCase().includes(kl)) return inp;
            }}
            return null;
        }}

        function setValue(el, val) {{
            const tag = el.tagName.toLowerCase();
            const type = (el.type || '').toLowerCase();

            // SELECT
            if (tag === 'select') {{
                const vl = val.toLowerCase();
                let matched = false;
                for (const opt of el.options) {{
                    if (opt.value.toLowerCase() === vl || opt.text.toLowerCase().includes(vl)) {{
                        el.value = opt.value;
                        matched = true;
                        break;
                    }}
                }}
                if (!matched) {{
                    // Try index if val is a number
                    const idx = parseInt(val);
                    if (!isNaN(idx) && idx < el.options.length) {{
                        el.selectedIndex = idx;
                        matched = true;
                    }}
                }}
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                return matched;
            }}

            // CHECKBOX
            if (type === 'checkbox') {{
                const shouldCheck = val === true || val === 'true' || val === '1' || val === 'on';
                if (el.checked !== shouldCheck) el.click();
                return true;
            }}

            // RADIO
            if (type === 'radio') {{
                const radios = document.querySelectorAll('[name="'+el.name+'"]');
                for (const r of radios) {{
                    const lbl = r.parentElement?.innerText?.toLowerCase() || '';
                    if (r.value.toLowerCase() === val.toLowerCase() || lbl.includes(val.toLowerCase())) {{
                        r.click();
                        return true;
                    }}
                }}
                return false;
            }}

            // TEXTAREA
            if (tag === 'textarea') {{
                el.value = val;
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                return true;
            }}

            // INPUT (text, email, password, tel, etc.)
            // Use React-compatible setter if available
            const nativeSet = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
            if (nativeSet) nativeSet.call(el, val);
            else el.value = val;
            el.dispatchEvent(new Event('input', {{bubbles: true}}));
            el.dispatchEvent(new Event('change', {{bubbles: true}}));
            return true;
        }}

        for (const [key, val] of Object.entries(fields)) {{
            const el = findField(key);
            if (el) {{
                if (setValue(el, val)) filled.push(key);
                else errors.push(key + ' (set failed)');
            }} else {{
                errors.push(key + ' (not found)');
            }}
        }}

        return JSON.stringify({{filled, errors}});
    ''')

def tool_submit(args):
    d = chrome()
    r = d.js('''
        let btn=document.querySelector('[type=submit],button[type=submit]');
        if(!btn)btn=document.querySelector('form')?.querySelector('button');
        if(btn){btn.click();return 'clicked'}
        const form=document.querySelector('form');
        if(form){form.submit();return 'submitted'}
        return '';
    ''')
    if not r: return 'No form or submit button found'
    time.sleep(2)
    return d.sanitize()

def tool_scroll(args):
    d = chrome()
    dy = int(args.get('amount', 500)) * (1 if args.get('direction', 'down') == 'down' else -1)
    d.js(f'window.scrollBy(0,{dy})')
    time.sleep(0.5)
    return d.sanitize()

def tool_screenshot(args):
    url = args.get('url', '')
    if url: chrome_go(url, 3)
    p = '/tmp/neo-screenshot.png'
    chrome().screenshot(p)
    return f'Screenshot: {p}'

def tool_wait(args):
    sel = args.get('selector', args.get('text', ''))
    if not sel: return 'selector or text required'
    d = chrome(); start = time.time(); timeout = int(args.get('wait', 10000)) / 1000
    while time.time() - start < timeout:
        found = d.js(f'const q={json.dumps(sel)};if(document.querySelector(q))return true;return Array.from(document.querySelectorAll("*")).some(e=>(e.innerText||"").includes(q))')
        if found: return d.sanitize()
        time.sleep(0.5)
    return f'Not found after {int(time.time()-start)}s: "{sel}"'

def tool_login(args):
    url, email, pw = args.get('url', ''), args.get('email', ''), args.get('password', '')
    if not all([url, email, pw]): return 'url, email, password required'
    d = chrome_go(url, 5)
    d.js(f'''const e={json.dumps(email)},p={json.dumps(pw)};
        const ei=document.querySelector('[type=email],[name=email],[name=username],[autocomplete=email]');
        const pi=document.querySelector('[type=password]');
        if(ei){{const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(s)s.call(ei,e);else ei.value=e;ei.dispatchEvent(new Event('input',{{bubbles:true}}))}}
        if(pi){{const s=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')?.set;if(s)s.call(pi,p);else pi.value=p;pi.dispatchEvent(new Event('input',{{bubbles:true}}))}}
    ''')
    time.sleep(1)
    d.js('document.querySelector("[type=submit],button[type=submit]")?.click()')
    time.sleep(5)
    return d.sanitize()

def tool_extract(args):
    t = args.get('type', 'links')
    d = chrome()
    if t == 'table':
        return d.js('const t=document.querySelector("table");if(!t)return "[]";return JSON.stringify(Array.from(t.querySelectorAll("tr")).map(r=>Array.from(r.querySelectorAll("th,td")).map(c=>c.innerText.trim())))')
    return d.js('return JSON.stringify(Array.from(document.querySelectorAll("a[href]")).slice(0,50).map(a=>({text:a.innerText.trim().substring(0,60),href:a.href})))')

# ── Chat ──

def _chat_ensure(platform, url, cookies):
    """Ensure chat platform is loaded. Cookies come from Ghost Chrome profile sync."""
    d = chrome()
    if platform not in _chrome_tabs:
        d.go(url); time.sleep(8)
        _chrome_tabs[platform] = True
        log(f'{platform}: {d.title}')
    return d

def _chat_wait_response(d, platform, user_msg, extract_js, count_js=None, max_wait=120):
    # Count responses BEFORE sending to detect new one
    before_count = 0
    if count_js:
        before_count = d.js(count_js) or 0

    prev = ''; stable = 0
    for i in range(max_wait):
        time.sleep(1)

        # Check if a NEW response appeared (count increased)
        if count_js and i < 30:
            current_count = d.js(count_js) or 0
            if current_count <= before_count and i < 15:
                continue  # No new response yet, keep waiting

        if i > 2:
            resp = d.js(extract_js)
            if resp and len(resp) > 3 and resp != user_msg:
                if resp == prev:
                    stable += 1
                    if stable >= 2: return save(resp, platform)
                else: stable = 0
                prev = resp

        if i > 0 and i % 15 == 0: log(f'{platform}: waiting... ({i}s)')
    return save(prev, platform) if prev else 'No response'

def tool_gpt(args):
    action = args.get('action', 'send')
    d = _chat_ensure('gpt', 'https://chatgpt.com', ['chatgpt.com', 'openai.com'])

    if action == 'read_last':
        return save(d.js('const m=document.querySelectorAll("[data-message-author-role=assistant]");return m.length?m[m.length-1].innerText:null') or 'No messages', 'gpt')
    if action == 'is_streaming':
        return json.dumps({'streaming': bool(d.js('return !!document.querySelector("[data-testid=stop-button]")')), 'open': True})
    if action == 'history':
        msgs = d.js(f'const m=[];document.querySelectorAll("[data-message-author-role]").forEach(e=>{{const r=e.getAttribute("data-message-author-role"),t=e.innerText?.trim()?.substring(0,300);if(t)m.push({{role:r,text:t}})}});return JSON.stringify(m.slice(-{int(args.get("count",5))}))')
        try:
            return '\n'.join(f'> {"YOU" if m["role"]=="user" else "GPT"}: {m["text"][:200]}' for m in json.loads(msgs))
        except: return msgs or 'No messages'

    # send
    msg = args.get('message', '')
    if not msg: return 'message required'
    # Focus the textarea and type via CDP key events (ProseMirror needs real keys)
    d.js('const el=document.getElementById("prompt-textarea");if(el){el.focus();el.click()}')
    time.sleep(0.3)
    d.key(msg)
    time.sleep(0.5)
    d.enter()
    log('GPT: sent')
    if not args.get('wait', True): return 'Sent.'
    return _chat_wait_response(d, 'gpt', msg,
        'const m=document.querySelectorAll("[data-message-author-role=assistant]");return m.length?m[m.length-1].innerText:null',
        'return document.querySelectorAll("[data-message-author-role=assistant]").length')

def tool_grok(args):
    action = args.get('action', 'send')
    d = _chat_ensure('grok', 'https://grok.com', ['x.com', 'grok.com'])

    if action == 'read_last':
        return save(d.js('const s=[".markdown","div.prose","article"];for(const q of s){const e=document.querySelectorAll(q);if(e.length>0)return e[e.length-1].innerText}return null') or 'No messages', 'grok')
    if action == 'is_streaming':
        return json.dumps({'streaming': bool(d.js('return !!document.querySelector("[class*=streaming],[class*=typing]")')), 'open': True})
    if action == 'history':
        return d.js('const m=document.querySelector("main")||document.body;return m.innerText?.substring(0,2000)') or 'No messages'

    # send
    msg = args.get('message', '')
    if not msg: return 'message required'
    d.js('const el=document.querySelector("div.query-bar p")||document.querySelector("div.query-bar");if(el){el.click();el.focus()}')
    time.sleep(0.3)
    d.key(msg); time.sleep(0.3); d.enter()
    log('Grok: sent')
    if not args.get('wait', True): return 'Sent.'
    return _chat_wait_response(d, 'grok', msg, f'''
        const userMsg={json.dumps(msg)};const main=document.querySelector("main")||document.body;
        const all=main.innerText||"";const idx=all.lastIndexOf(userMsg);
        if(idx>-1){{let a=all.substring(idx+userMsg.length).trim();
            a=a.replace(/^\\s*\\d+ sources?\\s*/i,"").replace(/Pregunta lo que quieras.*$/s,"");
            if(a.length>5)return a.trim()}}
        const s=[".markdown","div.prose","article"];
        for(const q of s){{const e=document.querySelectorAll(q);for(let i=e.length-1;i>=0;i--){{
            const t=e[i].innerText?.trim();if(t&&t.length>10&&!t.includes(userMsg))return t}}}}
        return null;
    ''')

def tool_status(args):
    return json.dumps({'chrome': _chrome is not None, 'tabs': list(_chrome_tabs.keys()), 'pids': list(_chrome_pids)}, indent=2)

# ── Plugins ──

def tool_plugin(args):
    from plugins import load_plugin, list_plugins, create_plugin, run_plugin

    action = args.get('action', 'run')

    if action == 'list':
        plugins = list_plugins()
        if not plugins:
            return 'No plugins found. Create one in ~/.neorender/plugins/*.yaml'
        lines = ['# Available Plugins\n']
        for p in plugins:
            inputs = ', '.join(p.get('inputs', []))
            lines.append(f'**{p["name"]}** — {p.get("description", "")}')
            if inputs:
                lines.append(f'  Inputs: {inputs}')
            lines.append(f'  Steps: {p.get("steps", 0)}')
            lines.append('')
        return '\n'.join(lines)

    elif action == 'create':
        name = args.get('name', '')
        desc = args.get('description', '')
        yaml_content = args.get('yaml', '')
        if not name or not yaml_content:
            return 'name and yaml required for create action'
        return create_plugin(name, desc, yaml_content)

    elif action == 'run':
        name = args.get('name', '')
        if not name:
            return 'name required. Use action=list to see available plugins.'

        plugin_data, err = load_plugin(name)
        if err:
            return err

        # Parse user inputs
        user_inputs = {}
        for key in plugin_data.get('inputs', {}):
            if key in args:
                val = args[key]
                # Parse list values
                if isinstance(val, str) and val.startswith('['):
                    try: val = json.loads(val)
                    except: val = [x.strip() for x in val.strip('[]').split(',')]
                user_inputs[key] = val

        # Execute with tool dispatch
        def dispatch(tool_name, tool_args):
            fn = DISPATCH.get(tool_name)
            if fn:
                return fn(tool_args)
            return f'Unknown tool: {tool_name}'

        try:
            result = run_plugin(plugin_data, user_inputs, dispatch)
            return save(result, f'plugin-{name}')
        except Exception as e:
            return f'Plugin error: {e}'

    return f'Unknown plugin action: {action}. Use: run, list, create'

# ── Cleanup ──
def cleanup():
    global _chrome
    if _chrome: _chrome.quit(); _chrome = None
    _kill_pids(); PID_FILE.unlink(missing_ok=True)
    # Remove SingletonLock so next launch doesn't fail
    ghost_dir = Path.home() / '.neorender' / 'ghost-profile'
    for lock in ['SingletonLock', 'SingletonSocket', 'SingletonCookie']:
        (ghost_dir / lock).unlink(missing_ok=True)
    log('Cleanup')

atexit.register(cleanup)
signal.signal(signal.SIGTERM, lambda *a: (cleanup(), sys.exit(0)))

# ── MCP Tools ──

TOOLS = [
    {"name": "browse", "description": "Fast HTTP browse (~1s). Returns page content with links and actions. Best for reading web pages.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string", "description": "URL to browse"}}, "required": ["url"]}},
    {"name": "search", "description": "Web search via DuckDuckGo (~1s).", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "num": {"type": "integer", "default": 10}}, "required": ["query"]}},
    {"name": "open", "description": "Open URL in Ghost Chrome (headless, CF bypass, ~5s). Use for SPAs, Cloudflare sites, or when browse returns empty.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}, "wait": {"type": "integer", "default": 5000, "description": "Wait ms after load"}}, "required": ["url"]}},
    {"name": "read", "description": "Extract text from page. With type: smart extraction (tweets, posts, comments, products, table). With selector: CSS extraction. Without: full page.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}, "type": {"type": "string", "enum": ["tweets", "posts", "comments", "products", "table"], "description": "Smart content extractor"}, "selector": {"type": "string", "description": "CSS selector fallback"}}}},
    {"name": "find", "description": "Find element by text, CSS selector, XPath, or ARIA role.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "by": {"type": "string", "enum": ["text", "css", "xpath", "role"], "default": "text"}}, "required": ["text"]}},
    {"name": "click", "description": "Click element by text content or CSS selector.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string", "description": "Text or CSS selector of element to click"}}, "required": ["text"]}},
    {"name": "type", "description": "Type text in input. Finds by: label text, placeholder, name, aria-label, type, or CSS selector.", "inputSchema": {"type": "object", "properties": {"selector": {"type": "string", "description": "Label text, placeholder, name, or CSS selector"}, "value": {"type": "string", "description": "Text to type"}}, "required": ["selector", "value"]}},
    {"name": "fill", "description": "Smart fill — handles inputs, textareas, selects, checkboxes, radios. Finds by label, placeholder, name, id. Use field labels as keys.", "inputSchema": {"type": "object", "properties": {"fields": {"type": "string", "description": "JSON: {\"Name\": \"John\", \"Email\": \"john@test.com\", \"Project type\": \"AI Agents\", \"Budget\": \"50K\"}"}, "url": {"type": "string", "description": "Optional URL to navigate first"}}, "required": ["fields"]}},
    {"name": "submit", "description": "Submit current form.", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "scroll", "description": "Scroll page.", "inputSchema": {"type": "object", "properties": {"direction": {"type": "string", "enum": ["up", "down"], "default": "down"}, "amount": {"type": "integer", "default": 500}}}},
    {"name": "screenshot", "description": "Capture screenshot of current page.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}}},
    {"name": "wait", "description": "Wait for element or text to appear on page.", "inputSchema": {"type": "object", "properties": {"selector": {"type": "string"}, "wait": {"type": "integer", "default": 10000}}, "required": ["selector"]}},
    {"name": "login", "description": "Automated login: fill email+password and submit.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}, "email": {"type": "string"}, "password": {"type": "string"}}, "required": ["url", "email", "password"]}},
    {"name": "extract", "description": "Extract structured data (tables or links).", "inputSchema": {"type": "object", "properties": {"type": {"type": "string", "enum": ["table", "links"], "default": "links"}}}},
    {"name": "gpt", "description": "ChatGPT. Send message or read. Actions: send (default), read_last, is_streaming, history.", "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}, "action": {"type": "string", "enum": ["send", "read_last", "is_streaming", "history"], "default": "send"}, "wait": {"type": "boolean", "default": True}, "count": {"type": "integer", "default": 5}, "raw": {"type": "boolean", "default": False}}}},
    {"name": "grok", "description": "Grok. Send message or read. Actions: send (default), read_last, is_streaming, history.", "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}, "action": {"type": "string", "enum": ["send", "read_last", "is_streaming", "history"], "default": "send"}, "wait": {"type": "boolean", "default": True}, "count": {"type": "integer", "default": 5}}}},
    {"name": "plugin", "description": "Run, list, or create browser plugins (reusable pipelines). Plugins are YAML files in ~/.neorender/plugins/. Actions: run (execute a plugin), list (show available), create (make new).", "inputSchema": {"type": "object", "properties": {"action": {"type": "string", "enum": ["run", "list", "create"], "default": "run"}, "name": {"type": "string", "description": "Plugin name"}, "description": {"type": "string"}, "yaml": {"type": "string", "description": "YAML content for create action"}}, "additionalProperties": True}},
    {"name": "status", "description": "Browser and chat session status.", "inputSchema": {"type": "object", "properties": {}}},
]

DISPATCH = {
    'browse': tool_browse, 'search': tool_search, 'open': tool_open, 'read': tool_read,
    'find': tool_find, 'click': tool_click, 'type': tool_type, 'fill': tool_fill,
    'submit': tool_submit, 'scroll': tool_scroll, 'screenshot': tool_screenshot,
    'wait': tool_wait, 'login': tool_login, 'extract': tool_extract,
    'gpt': tool_gpt, 'grok': tool_grok, 'plugin': tool_plugin, 'status': tool_status,
}

# ── MCP Protocol ──

def respond(id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "result": result}) + '\n'); sys.stdout.flush()

def respond_err(id, code, msg):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}) + '\n'); sys.stdout.flush()

def handle(req):
    method, params, id = req.get('method', ''), req.get('params', {}), req.get('id')
    if method == 'initialize':
        respond(id, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "neo-browser", "version": "3.0.0"}})
    elif method == 'tools/list':
        respond(id, {"tools": TOOLS})
    elif method == 'tools/call':
        name = params.get('name', '')
        args = params.get('arguments', {})
        fn = DISPATCH.get(name)
        if fn:
            try:
                result = fn(args)
                if result is None: result = ''
                text = result if isinstance(result, str) else json.dumps(result, ensure_ascii=False)
                # Guard: truncate if over 500KB to stay under websocket 1MB limit
                if len(text) > 500000:
                    text = text[:500000] + f'\n... (truncated from {len(text)} chars)'
                respond(id, {"content": [{"type": "text", "text": text}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})
        else:
            respond_err(id, -32601, f'Unknown tool: {name}')
    elif method == 'notifications/initialized':
        pass
    elif id is not None:
        respond_err(id, -32601, f'Unknown method: {method}')

log('NeoBrowser V3 started — 17 tools, Ghost Chrome headless, CF bypass')

for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try: handle(json.loads(line))
    except json.JSONDecodeError: log(f'JSON err: {line[:80]}')
    except Exception as e: log(f'Error: {e}')
