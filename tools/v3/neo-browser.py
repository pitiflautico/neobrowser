#!/usr/bin/env python3
"""
NeoBrowser V3 — AI Browser MCP Server.

Architecture:
  - Ghost Chrome: headless Chrome per MCP process (~/.neorender/ghost-{pid}/)
  - Session sync: cookies from real Chrome profile on startup (Google excluded)
  - Multi-tab: each site gets its own CDP tab (browsing, GPT, Grok)
  - Per-process isolation: no collisions between MCP instances

Tabs:
  - default: browsing tab (open, click, read, scroll, fill, etc.)
  - gpt: dedicated ChatGPT tab (persists across browse calls)
  - grok: dedicated Grok tab (persists across browse calls)
  New tabs created via CDP Target.createTarget, each with own WebSocket.

Tools (19):
  HTTP (no Chrome):
    BROWSE  — Fast HTTP fetch + parse (~1s). Best for reading pages.
    SEARCH  — Web search via DuckDuckGo HTML (~1s).

  Chrome browsing (default tab):
    OPEN    — Navigate to URL in Ghost Chrome (~5s). CF bypass, session.
    READ    — Extract text. type=tweets|posts|comments|products|table or CSS selector.
    FIND    — Find element by text, CSS, XPath, or ARIA role.
    CLICK   — Click element by text or CSS selector.
    TYPE    — Type in input (finds by label, placeholder, name, aria-label).
    FILL    — Smart fill: inputs, textareas, selects, checkboxes, radios.
    SUBMIT  — Submit form, returns sanitized page.
    SCROLL  — Scroll up/down, returns sanitized page.
    WAIT    — Wait for element/text, returns sanitized page.
    LOGIN   — Fill email+password and submit.
    EXTRACT — Extract tables or links as JSON.
    SCREENSHOT — Capture PNG.
    JS      — Execute arbitrary JavaScript.

  Chat (dedicated tabs):
    GPT     — ChatGPT. Actions: send, read_last, is_streaming, history.
    GROK    — Grok. Actions: send, read_last, is_streaming, history.

  Meta:
    PLUGIN  — Run/list/create YAML pipelines (~/.neorender/plugins/).
    STATUS  — Chrome state, active tabs, PIDs.

Session sync (on Chrome launch):
  1. Cookies: SQLite backup from real Chrome (WAL-safe). Google excluded.
  2. Local Storage: SPA auth tokens (X, ChatGPT, LinkedIn...).
  3. IndexedDB: some sites store auth here.

Install: pip install -e tools/v3/  →  command: neo-browser
Config:  {"command": "neo-browser", "env": {"NEOBROWSER_PROFILE": "Profile 24"}}
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

# API keys for direct chat (bypass browser, more reliable)
OPENAI_API_KEY = os.environ.get('OPENAI_API_KEY', '')
XAI_API_KEY = os.environ.get('XAI_API_KEY', '')

# Content processing via claude CLI (reduces tokens for main model)
CONTENT_PROCESS = os.environ.get('NEOBROWSER_CONTENT_PROCESS', '')  # set to '1' to enable
CONTENT_MAX_CHARS = 100000

# Cookie domain control: comma-separated list of domains to sync (empty = sync all except Google)
COOKIE_DOMAINS = [d.strip() for d in os.environ.get('NEOBROWSER_COOKIE_DOMAINS', '').split(',') if d.strip()]

# ── Security ──

def sanitize_unicode(text):
    """Strip invisible Unicode characters that could hide prompt injection."""
    if not text: return text
    import unicodedata
    text = unicodedata.normalize('NFKC', text)
    text = re.sub(r'[\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff\ue000-\uf8ff]', '', text)
    return text

def validate_url(url):
    """URL validation — block dangerous schemes, private IPs, and cloud metadata."""
    if not url: return False
    u = urllib.parse.urlparse(url)
    if u.scheme not in ('http', 'https'): return False
    if u.username or u.password: return False  # No credentials in URLs
    host = (u.hostname or '').lower()
    # Blocked hostnames (urlparse strips brackets from IPv6, so use bare forms)
    BLOCKED_HOSTS = {'localhost', '127.0.0.1', '0.0.0.0', '::', '::1',
                     'metadata.google.internal', 'metadata.internal'}
    if host in BLOCKED_HOSTS: return False
    # Robust IP check — covers IPv4 private, IPv6 loopback, IPv6-mapped
    # (::ffff:10.x.x.x), link-local, and reserved ranges
    import ipaddress
    try:
        ip = ipaddress.ip_address(host)
        if ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_reserved:
            return False
    except ValueError:
        pass  # Not an IP literal — hostname, allow
    return True

# Simple secret detection patterns
_SECRET_PATTERNS = [
    (r'sk-ant-api\w{20,}', 'Anthropic API key'),
    (r'sk-[a-zA-Z0-9]{20,}', 'OpenAI API key'),
    (r'AKIA[0-9A-Z]{16}', 'AWS Access Key'),
    (r'ghp_[a-zA-Z0-9]{36}', 'GitHub PAT'),
    (r'gho_[a-zA-Z0-9]{36}', 'GitHub OAuth'),
    (r'glpat-[a-zA-Z0-9\-_]{20,}', 'GitLab PAT'),
    (r'xoxb-[a-zA-Z0-9\-]+', 'Slack Bot Token'),
    (r'-----BEGIN (?:RSA |EC )?PRIVATE KEY-----', 'Private Key'),
]

def scan_secrets(text):
    """Scan text for leaked secrets. Returns list of pattern names or empty list."""
    if not text: return []
    found = []
    for pattern, name in _SECRET_PATTERNS:
        if re.search(pattern, text):
            found.append(name)
    return found

def _is_cf_challenge(html):
    """Detect Cloudflare challenge/block pages in HTTP responses."""
    if not html or len(html) < 100: return False
    signals = [
        'cf-browser-verification',     # CF JS challenge div
        'cf_chl_opt',                   # CF challenge options script
        'challenges.cloudflare.com',    # CF challenge iframe/script src
        'Just a moment...',             # CF waiting page title
        'Checking if the site connection is secure',  # CF interstitial text
        'Attention Required! | Cloudflare',           # CF block page title
        'cf-error-details',             # CF error page div
        'ray ID',                       # CF ray ID footer
    ]
    html_lower = html[:5000].lower()
    return sum(1 for s in signals if s.lower() in html_lower) >= 2

def error_response(code, message, url='', suggestion=''):
    """Structured error for agents to act on."""
    r = {'error': code, 'message': message}
    if url: r['url'] = url
    if suggestion: r['suggestion'] = suggestion
    log(f'ERROR [{code}]: {message}')
    return json.dumps(r)

# ── Page cache (LRU + TTL) ──

class PageCache:
    """Simple LRU cache with TTL. Thread-safe."""
    def __init__(self, max_items=50, ttl_s=900):  # 15min TTL, 50 entries
        self._cache = {}  # url → (timestamp, content)
        self._max = max_items
        self._ttl = ttl_s
        self._lock = threading.Lock()

    def get(self, url):
        with self._lock:
            if url in self._cache:
                ts, content = self._cache[url]
                if time.time() - ts < self._ttl:
                    log(f'Cache hit: {url[:60]}')
                    return content
                del self._cache[url]
        return None

    def put(self, url, content):
        with self._lock:
            # Evict oldest if full
            if len(self._cache) >= self._max:
                oldest = min(self._cache, key=lambda k: self._cache[k][0])
                del self._cache[oldest]
            self._cache[url] = (time.time(), content)

_page_cache = PageCache()

# ── Sequential execution for browser ops ──

_browser_lock = threading.RLock()  # Reentrant: plugin → dispatch_tool → lock OK

def sequential_browser(fn):
    """Decorator: serialize browser-mutating operations."""
    def wrapper(*args, **kwargs):
        with _browser_lock:
            return fn(*args, **kwargs)
    return wrapper

# ── Large result persistence ──

MAX_RESULT_CHARS = 100000

def persist_if_large(text, tag='result'):
    """If text exceeds MAX_RESULT_CHARS, save to disk and return path + preview."""
    if not text or len(text) <= MAX_RESULT_CHARS:
        return text
    ts = time.strftime('%Y%m%d-%H%M%S')
    p = RESPONSE_DIR / f'{tag}-{ts}.txt'
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)
    preview = text[:2000]
    log(f'Result too large ({len(text)} chars), saved to {p}')
    return f'{preview}\n\n... [{len(text)} chars total, saved to {p}]\nUse Read tool with offset/limit to read portions.'

NEOMODE_JS = '''
// ── Stealth: screen dimensions ──
Object.defineProperty(screen,'width',{get:()=>1920});
Object.defineProperty(screen,'height',{get:()=>1080});
Object.defineProperty(screen,'availWidth',{get:()=>1920});
Object.defineProperty(screen,'availHeight',{get:()=>1055});
Object.defineProperty(window,'outerHeight',{get:()=>1055});
Object.defineProperty(window,'innerHeight',{get:()=>968});

// ── Stealth: hide headless signals ──
Object.defineProperty(navigator,'webdriver',{get:()=>false});
Object.defineProperty(navigator,'plugins',{get:()=>[
    {name:'Chrome PDF Plugin',filename:'internal-pdf-viewer',description:'Portable Document Format',length:1},
    {name:'Chrome PDF Viewer',filename:'mhjfbmdgcfjbbpaeojofohoefgiehjai',description:'',length:1},
    {name:'Native Client',filename:'internal-nacl-plugin',description:'',length:2}
]});
Object.defineProperty(navigator,'languages',{get:()=>['es-ES','es','en-US','en']});
Object.defineProperty(navigator,'hardwareConcurrency',{get:()=>8});
Object.defineProperty(navigator,'deviceMemory',{get:()=>8});
Object.defineProperty(navigator,'maxTouchPoints',{get:()=>0});
window.chrome={runtime:{},loadTimes:function(){},csi:function(){}};
Object.defineProperty(navigator,'permissions',{get:()=>({
    query:p=>Promise.resolve({state:p.name==='notifications'?'denied':'granted',onchange:null})
})});
// WebGL vendor
const getParameter=WebGLRenderingContext.prototype.getParameter;
WebGLRenderingContext.prototype.getParameter=function(p){
    if(p===37445)return'Google Inc. (Apple)';
    if(p===37446)return'ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Pro, Unspecified Version)';
    return getParameter.call(this,p);
};

// Chat state tracker (no fetch interception — causes streaming hangs)
window.__neoChat = {response: '', done: false, ts: 0};

// ── Smart field detector: scoring + Shadow DOM + iframes + rich editors ──
window.__neoFind = function(hint) {
    const h = (hint || '').toLowerCase();
    // Gather all editable candidates via querySelectorAll (reliable, no generator)
    const SEL = 'input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([type=submit]):not([type=button]):not([type=file]):not([type=range]):not([type=color]),textarea,[contenteditable="true"],[role="textbox"],.ProseMirror,[data-slate-editor],[data-lexical-editor],.ql-editor';
    const candidates = document.querySelectorAll(SEL);
    let best = null, bestScore = -1;

    for (const el of candidates) {
        if (!el.isConnected || el.disabled || el.readOnly) continue;
        const r = el.getBoundingClientRect();
        if (r.width < 2 || r.height < 2) continue;
        try {
            const s = getComputedStyle(el);
            if (s.display==='none'||s.visibility==='hidden'||s.opacity==='0') continue;
        } catch { continue; }

        let score = 0;
        // Hint match (label, placeholder, name, id, aria-label)
        if (h) {
            const label = [el.placeholder, el.name, el.id, el.getAttribute?.('aria-label'),
                el.labels?.[0]?.innerText, el.closest?.('label')?.innerText,
                el.parentElement?.previousElementSibling?.innerText
            ].map(v=>(v||'').toLowerCase()).join(' ');
            if (label.includes(h)) score += 50;
        }
        // Editor bonus
        if (el.classList?.contains('ProseMirror') || el.getAttribute?.('data-slate-editor')) score += 20;
        if (el.getAttribute?.('role') === 'textbox') score += 15;
        // Focused
        if (document.activeElement === el) score += 30;
        // Size (larger = more important)
        score += Math.min(r.width * r.height / 10000, 15);
        // In viewport
        if (r.top >= 0 && r.bottom <= innerHeight) score += 5;

        if (score > bestScore) { bestScore = score; best = el; }
    }
    return best;
};
'''

SANITIZE_JS = '''(function(){
    const r={title:document.title,url:location.href};
    // Page type detection
    const u=location.href.toLowerCase();
    r.type=u.includes('login')||u.includes('sign_in')?'login':
           document.querySelectorAll('article').length>1?'feed':
           document.querySelector('article')?'article':'page';
    // Auth state
    r.auth=!!document.querySelector('[class*="avatar"],[class*="profile-photo"]')?'logged-in':'anonymous';
    // Main content — strip noise
    const main=document.querySelector('main,article,[role="main"],#content,.content')||document.body;
    const clone=main.cloneNode(true);
    ['script','style','nav','footer','header','aside','svg','noscript','iframe'].forEach(t=>clone.querySelectorAll(t).forEach(n=>n.remove()));
    r.text=clone.innerText.trim().replace(/\\n{3,}/g,'\\n\\n').substring(0,4000);
    // Forms — compact
    const forms=document.querySelectorAll('form');
    if(forms.length)r.forms=Array.from(forms).slice(0,3).map(f=>{
        const fields=Array.from(f.querySelectorAll('input:not([type=hidden]),textarea,select')).slice(0,10).map(i=>{
            const lbl=(i.labels?.[0]?.innerText||i.getAttribute('aria-label')||i.placeholder||i.name||'').trim();
            return lbl+':'+i.type;
        });
        const sub=f.querySelector('[type=submit],button[type=submit],button:not([type])');
        return fields.join(', ')+(sub?' ['+sub.innerText.trim()+']':'');
    });
    // Actions — clickable elements the LLM can use
    const seen=new Set();
    r.actions=Array.from(document.querySelectorAll('a[href],button,[role=button],[role=link],[role=tab]')).filter(el=>{
        const t=el.innerText.trim();
        if(!t||t.length>80||t.length<2||seen.has(t))return false;
        if(el.tagName==='A'&&el.href?.startsWith('javascript:'))return false;
        seen.add(t);return true;
    }).slice(0,20).map(el=>{
        const t=el.innerText.trim();
        const href=el.tagName==='A'?el.href:'';
        return href?t+' → '+href:t;
    });
    return JSON.stringify(r);
})()'''

# ── State ──
_chrome = None
_chrome_lock = threading.Lock()
_chrome_pids = set()

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
    """Headless Chrome with isolated tabs. Chat tabs get their own BrowserContext."""
    def __init__(self, proc, port, ws):
        self.proc = proc
        self.port = port
        self._tabs = {'default': ws}   # name → websocket
        self._active = 'default'
        self._id = 10
        self._keepalive = None         # background thread for chat keepalive

    @property
    def ws(self):
        return self._tabs[self._active]

    def tab(self, name, url=None):
        """Switch to tab by name. Creates it if it doesn't exist and url is given."""
        if name in self._tabs:
            self._active = name
            return self
        if not url:
            return None
        # Create tab WITH the target URL directly (not about:blank → navigate)
        # This is more reliable in headless because Chrome handles the initial
        # navigation internally, same as opening a new tab in a real browser.
        result = self._send('Target.createTarget', {'url': url})
        target_id = result.get('targetId', '')
        if not target_id:
            raise RuntimeError(f'Tab creation failed: {result}')
        # Poll for target to appear instead of fixed 1s sleep
        ws_url = None
        for _ in range(10):
            time.sleep(0.15)
            targets = json.loads(urllib.request.urlopen(
                f'http://127.0.0.1:{self.port}/json/list', timeout=5).read())
            ws_url = next((t['webSocketDebuggerUrl'] for t in targets if t.get('id') == target_id), None)
            if ws_url: break
        if not ws_url:
            raise RuntimeError(f'No WS for target {target_id}')
        ws = ws_sync.connect(ws_url, max_size=10_000_000, ping_interval=None)
        self._tabs[name] = ws
        self._active = name
        self._send('Page.enable')
        self._send('Network.enable')
        # Note: addScriptToEvaluateOnNewDocument won't apply to THIS navigation
        # (already started), but WILL apply to future navigations in this tab.
        self._send('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})
        self._send('Emulation.setDeviceMetricsOverride', {'width': 1920, 'height': 1080, 'deviceScaleFactor': 1, 'mobile': False})
        # Wait for page load
        for _ in range(30):
            time.sleep(0.5)
            state = self.js('return document.readyState')
            if state == 'complete': break
        # Inject NEOMODE_JS manually since addScriptToEvaluateOnNewDocument
        # missed the initial navigation
        self.js(NEOMODE_JS)
        log(f'Tab "{name}" → {self.js("return location.href")}')
        if name in ('gpt', 'grok'):
            self._start_keepalive()
        return self

    def _start_keepalive(self):
        """Background thread pings chat tabs every 15s to prevent GC/freeze."""
        if self._keepalive and self._keepalive.is_alive():
            return
        def _ping():
            while True:
                time.sleep(15)
                for name in list(self._tabs):
                    if name in ('gpt', 'grok') and name in self._tabs:
                        try:
                            old = self._active
                            self._active = name
                            self.js('1')  # no-op eval to keep tab alive
                            self._active = old
                        except: pass
        self._keepalive = threading.Thread(target=_ping, daemon=True)
        self._keepalive.start()
        log('Chat keepalive started')

    def js_async(self, code):
        """Execute JS with awaitPromise=true for async/Promise code."""
        self._id += 1
        self.ws.send(json.dumps({'id': self._id, 'method': 'Runtime.evaluate',
            'params': {'expression': code, 'returnByValue': True, 'awaitPromise': True}}))
        while True:
            data = json.loads(self.ws.recv(timeout=60))
            if data.get('id') == self._id:
                return data.get('result', {}).get('result', {}).get('value')

    def paste(self, text):
        """Paste text via clipboard — more reliable than key events for ProseMirror/contenteditable."""
        self.js(f'''
            const dt = new DataTransfer();
            dt.setData('text/plain', {json.dumps(text)});
            const el = document.activeElement;
            if (el) {{
                el.dispatchEvent(new ClipboardEvent('paste', {{clipboardData: dt, bubbles: true}}));
            }}
        ''')
        # Fallback: if paste event didn't work, set via execCommand
        current = self.js('return document.activeElement?.textContent || ""') or ''
        if text not in current:
            self.js(f'document.execCommand("insertText", false, {json.dumps(text)})')

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

    def accessibility(self):
        """Get page content via accessibility tree — token-efficient, sees what screen readers see."""
        self._send('Accessibility.enable')
        tree = self._send('Accessibility.getFullAXTree')
        nodes = tree.get('nodes', [])
        lines = []
        seen = set()
        for node in nodes:
            role = node.get('role', {}).get('value', '')
            name = node.get('name', {}).get('value', '')
            value = node.get('value', {}).get('value', '')
            if not name and not value:
                continue
            if role in ('generic', 'none', 'presentation', 'InlineTextBox', 'LineBreak', 'paragraph', 'Section', 'group'):
                continue
            text = name or value
            if text in seen: continue
            seen.add(text)
            if role == 'heading':
                level = node.get('properties', [{}])
                lines.append(f'# {text}')
            elif role == 'link':
                lines.append(f'[{text}]')
            elif role == 'button':
                lines.append(f'[btn: {text}]')
            elif role in ('textbox', 'searchbox', 'combobox'):
                lines.append(f'[input: {text}]')
            elif role == 'img':
                lines.append(f'[img: {text}]')
            elif role in ('StaticText', 'text'):
                lines.append(text)
            elif role in ('listitem', 'menuitem', 'option'):
                lines.append(f'- {text}')
            elif text.strip():
                lines.append(text)
        return '\n'.join(lines)

    def markdown(self):
        """Convert current page to clean Markdown."""
        return self.js('''
            function toMd(el, depth) {
                if (!el || depth > 15) return '';
                const tag = el.tagName?.toLowerCase() || '';
                const skip = ['script','style','nav','footer','header','aside','svg','noscript','iframe'];
                if (skip.includes(tag)) return '';

                // Text node
                if (el.nodeType === 3) return el.textContent || '';

                let md = '';
                const children = Array.from(el.childNodes).map(c => toMd(c, depth+1)).join('');

                switch(tag) {
                    case 'h1': return '\\n# ' + children.trim() + '\\n';
                    case 'h2': return '\\n## ' + children.trim() + '\\n';
                    case 'h3': return '\\n### ' + children.trim() + '\\n';
                    case 'h4': return '\\n#### ' + children.trim() + '\\n';
                    case 'h5': case 'h6': return '\\n##### ' + children.trim() + '\\n';
                    case 'p': return '\\n' + children.trim() + '\\n';
                    case 'br': return '\\n';
                    case 'hr': return '\\n---\\n';
                    case 'strong': case 'b': return '**' + children.trim() + '**';
                    case 'em': case 'i': return '*' + children.trim() + '*';
                    case 'code': return '`' + children.trim() + '`';
                    case 'pre': return '\\n```\\n' + children.trim() + '\\n```\\n';
                    case 'blockquote': return '\\n> ' + children.trim().replace(/\\n/g, '\\n> ') + '\\n';
                    case 'a': {
                        const href = el.getAttribute('href') || '';
                        const text = children.trim();
                        if (!text || !href || href.startsWith('javascript:')) return text;
                        return '[' + text + '](' + href + ')';
                    }
                    case 'img': {
                        const alt = el.getAttribute('alt') || '';
                        const src = el.getAttribute('src') || '';
                        return '![' + alt + '](' + src + ')';
                    }
                    case 'li': return '- ' + children.trim() + '\\n';
                    case 'ul': case 'ol': return '\\n' + children;
                    case 'table': return '\\n' + tableToMd(el) + '\\n';
                    case 'td': case 'th': return children.trim();
                    default: return children;
                }
            }

            function tableToMd(table) {
                const rows = Array.from(table.querySelectorAll('tr'));
                if (!rows.length) return '';
                const lines = [];
                rows.forEach((row, i) => {
                    const cells = Array.from(row.querySelectorAll('th,td')).map(c => c.innerText.trim());
                    lines.push('| ' + cells.join(' | ') + ' |');
                    if (i === 0) lines.push('| ' + cells.map(() => '---').join(' | ') + ' |');
                });
                return lines.join('\\n');
            }

            const main = document.querySelector('main,article,[role="main"],#content,.content') || document.body;
            let result = toMd(main, 0);
            // Clean up
            result = result.replace(/\\n{3,}/g, '\\n\\n').trim();
            return result.substring(0, 5000);
        ''')

    def screenshot(self, path):
        import base64
        r = self._send('Page.captureScreenshot', {'format': 'png'})
        with open(path, 'wb') as f: f.write(base64.b64decode(r.get('data', '')))

    def key(self, text):
        """Type text via CDP Input.insertText."""
        self._send('Input.insertText', {'text': text})

    def enter(self):
        self._send('Input.dispatchKeyEvent', {'type': 'keyDown', 'key': 'Enter', 'code': 'Enter', 'windowsVirtualKeyCode': 13, 'text': '\r'})
        self._send('Input.dispatchKeyEvent', {'type': 'keyUp', 'key': 'Enter', 'code': 'Enter'})

    @property
    def title(self): return self.js('document.title') or ''

    @property
    def url(self): return self.js('location.href') or ''

    def quit(self):
        for ws in self._tabs.values():
            try: ws.close()
            except: pass
        if self.proc:
            try: self.proc.kill()
            except: pass

    def sanitize(self):
        try:
            data = json.loads(self.js(SANITIZE_JS))
            parts = [f'=== {data["title"]} | {data["url"]} | {data.get("type","page")} ===']
            # Forms
            for f in data.get('forms', []):
                parts.append(f'Form: {f}')
            # Content
            text = data.get('text', '')
            if text: parts.append(text[:4000])
            # Actions (clickable)
            actions = data.get('actions', [])
            if actions:
                parts.append('\nActions: ' + ' | '.join(actions[:15]))
            return sanitize_unicode('\n'.join(parts))
        except:
            return sanitize_unicode(self.js("return document.body?.innerText?.substring(0,4000)") or self.title)


def _kill_pids():
    for pid in list(_chrome_pids):
        try: os.kill(pid, 9)
        except: _chrome_pids.discard(pid)

def chrome():
    global _chrome
    if _chrome:
        try:
            _chrome.tab('default')  # Always return on default tab
            result = _chrome.js('return document.readyState')
            if result: return _chrome
        except: pass
        try: _chrome.quit()
        except: pass
        _kill_pids()
        _chrome = None; _chrome_pids.clear(); time.sleep(1)

    with _chrome_lock:
        if _chrome: return _chrome
        for attempt in range(3):
            try:
                if attempt > 0: _kill_pids(); time.sleep(2)
                log('Launching Ghost Chrome...')
                import socket

                # Each MCP instance gets its own Chrome profile (no collisions)
                ghost_dir = Path.home() / '.neorender' / f'ghost-{os.getpid()}'
                ghost_default = ghost_dir / 'Default'
                ghost_default.mkdir(parents=True, exist_ok=True)

                # Sync cookies from real Chrome
                real_profile = Path.home() / 'Library' / 'Application Support' / 'Google' / 'Chrome' / PROFILE
                if real_profile.exists():
                    _sync_session(real_profile, ghost_default)

                s = socket.socket(); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.close()
                proc = subprocess.Popen([CHROME_BIN, f'--remote-debugging-port={port}',
                    f'--user-data-dir={str(ghost_dir)}', '--headless=new', '--no-first-run',
                    '--disable-background-networking', '--disable-dev-shm-usage',
                    '--disable-blink-features=AutomationControlled',
                    '--window-size=1920,1080', f'--user-agent={CHROME_UA}', 'about:blank'],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                _chrome_pids.add(proc.pid)
                # Poll for Chrome to be ready instead of fixed 2s sleep
                for _ in range(20):
                    time.sleep(0.15)
                    try:
                        urllib.request.urlopen(f'http://127.0.0.1:{port}/json/version', timeout=2)
                        break
                    except: pass

                targets = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=10).read())
                ws_url = [t['webSocketDebuggerUrl'] for t in targets if t['type'] == 'page'][0]
                ws = ws_sync.connect(ws_url, max_size=10_000_000, ping_interval=None)
                _chrome = GhostChrome(proc, port, ws)
                _chrome._send('Page.enable'); _chrome._send('Network.enable')
                _chrome._send('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_JS})
                _chrome._send('Emulation.setDeviceMetricsOverride', {'width': 1920, 'height': 1080, 'deviceScaleFactor': 1, 'mobile': False})

                PID_FILE.parent.mkdir(parents=True, exist_ok=True)
                PID_FILE.write_text(json.dumps(list(_chrome_pids)))
                log(f'Ghost Chrome ready (port={port}, pid={proc.pid})')
                return _chrome
            except Exception as e:
                log(f'Chrome attempt {attempt+1} failed: {e}'); _chrome = None
        raise RuntimeError('Chrome failed after 3 attempts')


def _sync_session(src_profile, dst_profile):
    """Sync cookies and session data from real Chrome profile to Ghost profile.
    Both profiles use the same macOS Keychain encryption key, so cookies decrypt fine."""
    import shutil, sqlite3
    synced = []

    # 1. Cookies DB — selective sync (exclude Google to prevent session invalidation)
    # Google detects duplicate sessions from headless Chrome and logs out the real browser.
    EXCLUDED_DOMAINS = ('.google.com', '.google.es', '.googleapis.com', '.gstatic.com',
                        '.youtube.com', '.accounts.google.com', '.gmail.com')
    src_cookies = src_profile / 'Cookies'
    dst_cookies = dst_profile / 'Cookies'
    if src_cookies.exists() and src_cookies.stat().st_size > 0:
        try:
            conn_src = sqlite3.connect(f'file:{src_cookies}?mode=ro&nolock=1', uri=True)
            conn_dst = sqlite3.connect(str(dst_cookies))
            # Copy schema first
            conn_src.backup(conn_dst)
            if COOKIE_DOMAINS:
                # Allowlist mode: keep only cookies matching specified domains, delete everything else
                log(f'Cookie sync: domain allowlist active ({len(COOKIE_DOMAINS)} domains)')
                keep_conditions = ' OR '.join('host_key LIKE ?' for _ in COOKIE_DOMAINS)
                keep_params = [f'%{d}%' for d in COOKIE_DOMAINS]
                deleted = conn_dst.execute(
                    f'DELETE FROM cookies WHERE NOT ({keep_conditions})', keep_params
                ).rowcount
                count = conn_dst.execute('SELECT COUNT(*) FROM cookies').fetchone()[0]
                conn_dst.commit()
                conn_dst.close(); conn_src.close()
                synced.append(f'Cookies ({count} kept, {deleted} outside allowlist removed)')
            else:
                # Default: exclude Google domains to prevent session invalidation
                excluded = ' OR '.join('host_key LIKE ?' for _ in EXCLUDED_DOMAINS)
                excluded_params = [f'%{d}' for d in EXCLUDED_DOMAINS]
                deleted = conn_dst.execute(
                    f'DELETE FROM cookies WHERE {excluded}', excluded_params
                ).rowcount
                count = conn_dst.execute('SELECT COUNT(*) FROM cookies').fetchone()[0]
                conn_dst.commit()
                conn_dst.close(); conn_src.close()
                synced.append(f'Cookies ({count} kept, {deleted} Google excluded)')
        except Exception as e:
            log(f'Cookie sync failed: {e}')

    # 2. Local Storage (SPA auth tokens — X, ChatGPT, LinkedIn, etc.)
    for dirname in ['Local Storage', 'Session Storage']:
        src_dir = src_profile / dirname
        dst_dir = dst_profile / dirname
        if src_dir.exists():
            try:
                if dst_dir.exists(): shutil.rmtree(str(dst_dir))
                shutil.copytree(str(src_dir), str(dst_dir), dirs_exist_ok=True)
                synced.append(dirname)
            except Exception as e:
                log(f'Failed to sync {dirname}: {e}')

    # 3. IndexedDB (some sites store auth here)
    src_idb = src_profile / 'IndexedDB'
    dst_idb = dst_profile / 'IndexedDB'
    if src_idb.exists():
        try:
            if dst_idb.exists(): shutil.rmtree(str(dst_idb))
            shutil.copytree(str(src_idb), str(dst_idb), dirs_exist_ok=True)
            synced.append('IndexedDB')
        except Exception as e:
            log(f'Failed to sync IndexedDB: {e}')

    if synced:
        log(f'Session sync from {src_profile.name}: {", ".join(synced)}')
    else:
        log(f'Session sync: nothing synced from {src_profile.name}')

def _resync_cookies():
    """Re-sync cookies from real Chrome profile into running Ghost Chrome via CDP."""
    import sqlite3
    real_cookies = Path.home() / 'Library' / 'Application Support' / 'Google' / 'Chrome' / PROFILE / 'Cookies'
    if not real_cookies.exists(): return 0
    d = chrome()
    try:
        conn = sqlite3.connect(f'file:{real_cookies}?mode=ro&nolock=1', uri=True)
        rows = conn.execute(
            'SELECT host_key, name, path, is_secure, expires_utc, is_httponly FROM cookies'
        ).fetchall()
        # We can't read encrypted values directly via SQL, but we CAN
        # re-sync the cookie DB file and reload the page
        conn.close()
    except:
        pass
    # Copy fresh cookies DB to ghost profile
    ghost_cookies = Path.home() / '.neorender' / f'ghost-{os.getpid()}' / 'Default' / 'Cookies'
    try:
        conn_src = sqlite3.connect(f'file:{real_cookies}?mode=ro&nolock=1', uri=True)
        conn_dst = sqlite3.connect(str(ghost_cookies))
        conn_src.backup(conn_dst)
        conn_dst.close(); conn_src.close()
        log('Re-synced cookies from real Chrome')
        return 1
    except Exception as e:
        log(f'Cookie re-sync failed: {e}')
        return 0

def _is_login_wall(d):
    """Detect if current page is a login/auth wall."""
    check = d.js('''
        const url = location.href.toLowerCase();
        const text = (document.body?.innerText || '').toLowerCase().substring(0, 2000);
        const loginUrls = ['login', 'signin', 'sign-in', 'sign_in', 'auth', 'sso', 'oauth', 'accounts'];
        const loginText = ['sign in', 'log in', 'iniciar sesión', 'inicia sesión', 'create an account', 'join now'];
        const urlMatch = loginUrls.some(k => url.includes(k));
        const textMatch = loginText.some(k => text.includes(k));
        const hasLoginForm = !!document.querySelector('[type=password], [autocomplete=password]');
        return JSON.stringify({urlMatch, textMatch, hasLoginForm, url: location.href});
    ''')
    try:
        info = json.loads(check)
        return info.get('hasLoginForm') or (info.get('urlMatch') and info.get('textMatch'))
    except:
        return False

def chrome_go(url, wait_s=5):
    """Navigate default tab to URL. Chat tabs stay untouched."""
    d = chrome()
    d.go(url)
    # Poll readyState instead of fixed sleep
    deadline = time.time() + wait_s
    while time.time() < deadline:
        time.sleep(0.15)
        if d.js('return document.readyState') == 'complete':
            time.sleep(0.3)  # Brief settle for JS frameworks
            break

    # Check for login wall → resync cookies and retry
    if _is_login_wall(d):
        log(f'Login wall detected, re-syncing cookies...')
        if _resync_cookies():
            # Restart Chrome to pick up new cookies from DB
            d.go(url); time.sleep(wait_s)
            if _is_login_wall(d):
                log('Still on login wall after resync')
    return d

def save(text, tag='response'):
    if not text: return 'No content'
    secrets = scan_secrets(text)
    if secrets:
        log(f'WARNING: potential secrets detected in output: {", ".join(secrets)}')
    if len(text) <= 4000: return text
    ts = time.strftime('%Y%m%d-%H%M%S')
    p = RESPONSE_DIR / f'{tag}-{ts}.md'
    p.write_text(text)
    return text[:4000] + f'\n... [{len(text)} chars total → {p}]'

def process_content(text, prompt='You are a content extractor. Output ONLY the extracted data, no commentary. Extract the main content as clean structured text. Remove navigation, ads, footers, cookie banners, boilerplate. Keep titles, links, dates, authors, numbers. Do not interpret or analyze — just extract and structure.'):
    """Pass web content through claude -p to extract only relevant info."""
    if not CONTENT_PROCESS or len(text) < 500:
        return text

    truncated = text[:CONTENT_MAX_CHARS]
    try:
        result = subprocess.run(
            ['claude', '-p', '--model', 'haiku', f'{prompt}\n\n---\n\n{truncated}'],
            capture_output=True, text=True, timeout=30
        )
        content = result.stdout.strip()
        if content and len(content) > 50:
            log(f'Content processed via claude: {len(text)} → {len(content)} chars')
            return content
    except FileNotFoundError:
        log('Content processing: claude CLI not found')
    except subprocess.TimeoutExpired:
        log('Content processing: timeout')
    except Exception as e:
        log(f'Content processing failed: {e}')

    return text

# ── Tool definitions ──

TOOLS = {}

def tool_def(name, description, schema, read_only=True, concurrent=True, max_result=0):
    """Register a tool with metadata. Decorator."""
    def decorator(fn):
        TOOLS[name] = {
            'name': name,
            'description': description,
            'schema': schema,
            'read_only': read_only,
            'concurrent': concurrent,
            'max_result': max_result,
            'fn': fn,
        }
        return fn
    return decorator

# ── Tool implementations ──

@tool_def('browse', 'Fetch and parse a web page (fast HTTP, falls back to Chrome)', {'url': 'required', 'selector': 'optional CSS selector'}, read_only=True, concurrent=True, max_result=100000)
def tool_browse(args):
    url = args.get('url', '')
    if not url: return 'url required'
    if not validate_url(url):
        return error_response('url_blocked', f'URL blocked by security policy: {url}', suggestion='Only public HTTP/HTTPS URLs are allowed')
    cached = _page_cache.get(url)
    if cached: return cached
    out, ms = fast('see', url)
    if len(out) > 200:
        log(f'V1 browse: {ms}ms')
        result = sanitize_unicode(process_content(out))
        _page_cache.put(url, result)
        return result
    # Try raw HTTP before expensive Chrome fallback
    try:
        req = urllib.request.Request(url, headers={'User-Agent': CHROME_UA})
        resp = urllib.request.urlopen(req, timeout=10)
        body = resp.read().decode('utf-8', errors='replace')
        if _is_cf_challenge(body):
            log(f'Cloudflare challenge detected, skipping HTTP → Chrome')
        elif len(body) > 200:
            text = re.sub(r'<[^>]+>', ' ', body)
            text = re.sub(r'\s+', ' ', text).strip()
            if len(text) > 100:
                log(f'HTTP fallback: {len(text)} chars')
                result = sanitize_unicode(process_content(text[:5000]))
                _page_cache.put(url, result)
                return result
    except: pass
    log('HTTP fallback insufficient, Chrome fallback...')
    d = chrome_go(url)
    result = sanitize_unicode(process_content(d.sanitize()))
    _page_cache.put(url, result)
    return result

@tool_def('search', 'Search DuckDuckGo', {'query': 'required'}, read_only=True, concurrent=True)
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
            if url and title and validate_url(url):
                results.append(f'{title}\n  {url}')
                if len(results) >= num: break
        return '\n\n'.join(results) if results else 'No results'
    except Exception as e:
        return f'Search error: {e}'

@tool_def('open', 'Open URL in Chrome tab', {'url': 'required', 'tab': 'optional tab name'}, read_only=False, concurrent=False)
def tool_open(args):
    url = args.get('url', '')
    if not url: return 'url required'
    if not validate_url(url):
        log(f'WARNING: navigating to potentially unsafe URL: {url}')
    d = chrome_go(url, int(args.get('wait', 5000)) / 1000)
    return process_content(d.sanitize())

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

@tool_def('read', 'Read current page content as markdown/structured text', {'selector': 'optional CSS selector', 'mode': 'optional: markdown|a11y|tweets|posts|tables'}, read_only=True, concurrent=True, max_result=100000)
def tool_read(args):
    url = args.get('url', '')
    selector = args.get('selector', '')
    content_type = args.get('type', '') or args.get('mode', '')
    if url:
        if not selector and not content_type:
            out, ms = fast('see', url)
            if len(out) > 100:
                log(f'V1 read: {ms}ms')
                return sanitize_unicode(out[:3000])
        chrome_go(url, 3)
    d = chrome()

    # Smart extractor by content type
    if content_type:
        ct = content_type.lower()
        # Built-in extractors
        if ct == 'markdown':
            return process_content(save(d.markdown() or 'No content', 'read-md'))
        if ct == 'accessibility':
            return save(d.accessibility() or 'No content', 'read-a11y')
        # JS-based extractors
        js = SMART_EXTRACTORS.get(ct)
        if not js:
            types = list(SMART_EXTRACTORS.keys()) + ['markdown', 'accessibility']
            return f'Unknown type: {content_type}. Available: {", ".join(types)}'
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

@tool_def('find', 'Find elements on page by text, role, or selector', {'text': 'optional', 'role': 'optional', 'selector': 'optional'}, read_only=True, concurrent=True)
def tool_find(args):
    text = args.get('text', args.get('selector', ''))
    by = args.get('by', 'text')
    if not text: return 'text or selector required'
    return chrome().js(f'''
        const q={json.dumps(text)},by={json.dumps(by)};let els=[];
        if(by==='css')els=Array.from(document.querySelectorAll(q));
        else if(by==='xpath'){{const r=document.evaluate(q,document,null,5,null);let n;while(n=r.iterateNext())els.push(n)}}
        else{{const ql=q.toLowerCase();
            const SEL='a,button,input,select,textarea,label,h1,h2,h3,h4,p,li,td,th,span,[role=button],[role=link],[role=tab],[aria-label]';
            els=Array.from(document.querySelectorAll(SEL)).filter(e=>{{
                const aria=(e.getAttribute('aria-label')||'').toLowerCase();
                const ph=(e.placeholder||'').toLowerCase();
                const t=(e.innerText||'').toLowerCase();
                return aria.includes(ql)||ph.includes(ql)||(t.includes(ql)&&t.length<300);
            }});
            els.sort((a,b)=>(a.innerText||'').length-(b.innerText||'').length)}}
        return JSON.stringify(els.slice(0,5).map((e,i)=>({{
            index:i,tag:e.tagName.toLowerCase(),text:(e.innerText||'').substring(0,80),
            clickable:e.tagName==='A'||e.tagName==='BUTTON'||e.getAttribute('role')==='button'
        }})));
    ''') or '[]'

@tool_def('click', 'Click an element', {'text': 'optional', 'selector': 'optional', 'index': 'optional'}, read_only=False, concurrent=False)
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
    if clicked: time.sleep(0.5)  # Brief wait for click handler
    return f'Clicked "{text}"\n\n{d.sanitize()}' if clicked else f'Not found: "{text}"'

@tool_def('type', 'Type text into focused element', {'text': 'required'}, read_only=False, concurrent=False)
def tool_type(args):
    """Smart type — uses __neoFind detector."""
    sel = args.get('selector', args.get('text', ''))
    val = args.get('value', '')
    if not sel or not val: return 'selector and value required'
    d = chrome()
    found = d.js(f'''
        const el = window.__neoFind?.({json.dumps(sel)}) || document.querySelector({json.dumps(sel)});
        if (el) {{
            el.focus(); el.click();
            // Clear only standard inputs, not contentEditable (ProseMirror)
            if (el.tagName==='INPUT'||el.tagName==='TEXTAREA') el.value = '';
            el.dispatchEvent(new Event('focus', {{bubbles:true}}));
            return JSON.stringify({{found: true, tag: el.tagName, name: el.name||''}});
        }}
        return JSON.stringify({{found: false}});
    ''')
    try: info = json.loads(found) if found else {'found': False}
    except: info = {'found': False}

    if not info.get('found'):
        return f'Not found: "{sel}"'

    d.key(val)
    return json.dumps({'typed': True, 'value': val, 'field': info})

@tool_def('fill', 'Fill a form field', {'selector': 'optional', 'text': 'optional', 'value': 'required'}, read_only=False, concurrent=False)
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
                const vl = val.toLowerCase();
                for (const r of radios) {{
                    const parentText = r.parentElement?.innerText?.toLowerCase() || '';
                    const labelFor = r.id ? (document.querySelector('label[for="'+r.id+'"]')?.innerText?.toLowerCase() || '') : '';
                    if (r.value.toLowerCase() === vl || parentText.includes(vl) || labelFor.includes(vl)) {{
                        r.checked = true;
                        r.dispatchEvent(new Event('change', {{bubbles: true}}));
                        r.dispatchEvent(new Event('input', {{bubbles: true}}));
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
            const el = window.__neoFind?.(key) || document.querySelector('[name="'+key+'"]');
            if (el) {{
                if (setValue(el, val)) filled.push(key);
                else errors.push(key + ' (set failed)');
            }} else {{
                errors.push(key + ' (not found)');
            }}
        }}

        return JSON.stringify({{filled, errors}});
    ''')

@tool_def('submit', 'Submit a form', {'selector': 'optional'}, read_only=False, concurrent=False)
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
    time.sleep(1)
    return d.sanitize()

@tool_def('scroll', 'Scroll the page', {'direction': 'optional: up|down', 'amount': 'optional pixels'}, read_only=False, concurrent=False)
def tool_scroll(args):
    d = chrome()
    dy = int(args.get('amount', 500)) * (1 if args.get('direction', 'down') == 'down' else -1)
    d.js(f'window.scrollBy(0,{dy})')
    return d.sanitize()

@tool_def('screenshot', 'Take a screenshot of current page', {}, read_only=True, concurrent=True)
def tool_screenshot(args):
    url = args.get('url', '')
    if url: chrome_go(url, 3)
    p = '/tmp/neo-screenshot.png'
    chrome().screenshot(p)
    return f'Screenshot: {p}'

@tool_def('wait', 'Wait for element or condition', {'selector': 'optional', 'text': 'optional', 'timeout': 'optional ms'}, read_only=True, concurrent=True)
def tool_wait(args):
    sel = args.get('selector', args.get('text', ''))
    if not sel: return 'selector or text required'
    d = chrome(); start = time.time(); timeout = int(args.get('wait', 10000)) / 1000
    while time.time() - start < timeout:
        found = d.js(f'const q={json.dumps(sel)};if(document.querySelector(q))return true;return Array.from(document.querySelectorAll("*")).some(e=>(e.innerText||"").includes(q))')
        if found: return d.sanitize()
        time.sleep(0.5)
    return f'Not found after {int(time.time()-start)}s: "{sel}"'

@tool_def('login', 'Log into a website using stored session', {'url': 'required'}, read_only=False, concurrent=False)
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
    time.sleep(0.5)
    d.js('document.querySelector("[type=submit],button[type=submit]")?.click()')
    time.sleep(3)
    return d.sanitize()

@tool_def('extract', 'Extract links or table data from current page', {'type': 'optional: links|tables'}, read_only=True, concurrent=True, max_result=100000)
def tool_extract(args):
    t = args.get('type', 'links')
    d = chrome()
    if t == 'table':
        return d.js(SMART_EXTRACTORS['table']) or ''  # Reuse smart extractor, no duplication
    result = d.js('''
        const seen=new Set();
        const links=Array.from(document.querySelectorAll("a[href]")).filter(a=>{
            const t=a.innerText.trim();
            if(!t||t.length<2||t.length>80||seen.has(t)||a.href.startsWith("javascript:"))return false;
            seen.add(t);return true;
        }).slice(0,30).map(a=>a.innerText.trim()+" → "+a.href).join("\\n");
        return links || "No links found";
    ''') or 'No links found'
    return result

# ── Chat ──

class ChatPipeline:
    """Closed pipeline for chat platforms. Each step verifies before proceeding."""

    def __init__(self, platform, url):
        self.platform = platform
        self.url = url
        self.conv_url = None  # Current conversation URL (e.g. chatgpt.com/c/xxx)
        self.d = None
        self.max_retries = 2
        self.last_error = None

    def _resync_and_reload(self, browser):
        """Kill Ghost Chrome, re-sync cookies from real Chrome, relaunch.
        Chrome caches cookies in memory — file copy alone doesn't work at runtime.
        Only a full restart makes Chrome read the fresh cookie DB."""
        global _chrome
        log(f'{self.platform}: restarting Ghost Chrome for fresh cookies')
        try:
            # Kill current Ghost Chrome
            try: browser.quit()
            except: pass
            _kill_pids()
            _chrome = None; _chrome_pids.clear()
            time.sleep(1)
            # chrome() will re-sync cookies and relaunch
            d = chrome()
            self.d = d
            self.conv_url = None
            d.go(self.url)
            for _ in range(20):
                time.sleep(0.5)
                if d.js('return document.readyState') == 'complete': break
            time.sleep(1)
            return True
        except Exception as e:
            log(f'{self.platform}: restart failed: {e}')
            return False

    def ensure(self):
        """Step 1: Navigate default tab to chat platform."""
        self.last_error = None
        d = chrome()
        target = self.conv_url or self.url
        domain = self.url.split('/')[2]
        current = d.js('return location.href') or ''

        # Navigate if not already on the platform
        if domain not in current:
            log(f'{self.platform}: navigating to {target}')
            d.go(target)
            for _ in range(30):
                time.sleep(0.5)
                if d.js('return document.readyState') == 'complete':
                    cur = d.js('return location.href') or ''
                    if domain in cur: break
            time.sleep(1)
        elif self.conv_url and self.conv_url not in current:
            log(f'{self.platform}: navigating to conversation {self.conv_url}')
            d.go(self.conv_url)
            time.sleep(3)

        # Check for error state
        error = d.js('return document.body?.innerText?.includes("Something went wrong")')
        if error:
            log(f'{self.platform}: error state, navigating fresh')
            self.conv_url = None
            d.go(self.url); time.sleep(5)

        # Detect login/captcha/rate limit
        page_text = d.js('return document.body?.innerText?.substring(0,500)') or ''
        login_signals = ['log in', 'sign in', 'sign up', 'create account', 'iniciar sesión', 'inicia sesión']
        if any(s in page_text.lower() for s in login_signals):
            log(f'{self.platform}: login wall detected, attempting cookie re-sync')
            if self._resync_and_reload(d):
                # Check again after re-sync
                time.sleep(3)
                page_text = d.js('return document.body?.innerText?.substring(0,500)') or ''
                if not any(s in page_text.lower() for s in login_signals):
                    log(f'{self.platform}: re-sync fixed login wall')
                else:
                    log(f'{self.platform}: re-sync did not fix login wall')
                    self.last_error = error_response('login_wall', 'Login required (re-sync failed)', suggestion='Log into ChatGPT in your real Chrome browser, then restart NeoBrowser')
                    return False
            else:
                self.last_error = error_response('login_wall', 'Login required', suggestion='Log into ChatGPT in your real Chrome browser, then restart NeoBrowser')
                return False
        if any(s in page_text.lower() for s in ['captcha', 'verify you are human', 'cloudflare']):
            log(f'{self.platform}: captcha detected')
            self.last_error = error_response('captcha', 'Captcha or verification required', suggestion='Try again later or solve manually')
            return False
        if any(s in page_text.lower() for s in ['rate limit', 'too many requests', 'try again later']):
            log(f'{self.platform}: rate limited')
            self.last_error = error_response('rate_limit', 'Rate limited by platform', suggestion='Wait and retry')
            return False

        # Inject NEOMODE_JS if not present
        if not d.js('return typeof window.__neoFind === "function"'):
            d.js(NEOMODE_JS)
        self.d = d
        return True

    def verify_ready(self):
        """Step 2: No pending response, input field is available."""
        d = self.d
        if not d.js('return typeof window.__neoFind === "function"'):
            d.js(NEOMODE_JS)
        # Wait for any in-progress streaming to finish
        if d.js('return !!document.querySelector("[data-testid=stop-button]")'):
            log(f'{self.platform}: streaming in progress, waiting...')
            for _ in range(60):
                time.sleep(0.5)
                if not d.js('return !!document.querySelector("[data-testid=stop-button]")'): break
        # Check input exists
        has_input = d.js('return !!(document.getElementById("prompt-textarea") || window.__neoFind?.())')
        if not has_input:
            log(f'{self.platform}: input not found, reloading')
            d.go(self.url); time.sleep(5)
            has_input = d.js('return !!window.__neoFind?.()')
            if not has_input:
                log(f'{self.platform}: input still not found after reload')
        return bool(has_input)

    def send(self, msg):
        """Step 3: Type message and send."""
        d = self.d
        # Capture both count AND text of last assistant message (to detect stale responses)
        self._msg_count_before = int(d.js(
            'return document.querySelectorAll("[data-message-author-role=assistant]").length'
        ) or 0)
        self._last_text_before = d.js(
            'const m=document.querySelectorAll("[data-message-author-role=assistant]");return m.length?m[m.length-1].innerText?.substring(0,200):""'
        ) or ''
        log(f'{self.platform}: before: {self._msg_count_before} msgs, last="{self._last_text_before[:50]}"')
        # Focus textarea
        d.js('const el=document.getElementById("prompt-textarea")||window.__neoFind?.();if(el){el.focus();el.click()}')
        time.sleep(0.1)
        # Type: try CDP insertText, fallback to direct DOM manipulation
        d.key(msg)
        typed = d.js('const el=document.activeElement;return(el?.innerText||el?.value||"").length>0')
        if not typed:
            log(f'{self.platform}: key() did not populate input, falling back to innerText injection')
            d.js(f'''const el=document.getElementById("prompt-textarea")||window.__neoFind?.();
                if(el){{el.focus();el.innerText={json.dumps(msg)};el.dispatchEvent(new Event("input",{{bubbles:true}}))}}''')
        else:
            log(f'{self.platform}: key() successfully populated input')
        time.sleep(0.1)
        # Verify text is in the input before sending
        content = d.js('const el=document.getElementById("prompt-textarea")||window.__neoFind?.();return el?.innerText||""')
        if not content or len(content) < 3:
            log(f'{self.platform}: WARNING — text not in input, send may fail')
        # Send: Enter + send button click (covers all cases)
        user_count_before = int(d.js(
            'return document.querySelectorAll("[data-message-author-role=user]").length'
        ) or 0)
        d.enter()
        d.js('const b=document.querySelector("[data-testid=send-button]");if(b&&!b.disabled)b.click()')
        # Verify: wait up to 3s for user message to appear in DOM
        sent = False
        for _ in range(6):
            time.sleep(0.5)
            user_count = int(d.js(
                'return document.querySelectorAll("[data-message-author-role=user]").length'
            ) or 0)
            if user_count > user_count_before:
                sent = True
                break
            # Also check if stop button appeared (ChatGPT started processing)
            if d.js('return !!document.querySelector("[data-testid=stop-button]")'):
                sent = True
                break
        if sent:
            log(f'{self.platform}: sent verified ({len(msg)} chars)')
        else:
            log(f'{self.platform}: WARNING — send not verified, message may not have been sent')
        self._send_verified = sent
        return True

    def check_response(self):
        """Check response state. Non-blocking. Returns dict with granular status."""
        d = self.d
        if not d: return None
        state = d.js('''
            const msgs = document.querySelectorAll("[data-message-author-role=assistant]");
            const userMsgs = document.querySelectorAll("[data-message-author-role=user]");
            const count = msgs.length;
            const last = count ? msgs[msgs.length-1] : null;
            const text = last?.innerText || "";
            const stopBtn = !!document.querySelector("[data-testid=stop-button]");
            const streaming = !!document.querySelector(".result-streaming,[aria-busy=true]");
            const thinking = !!document.querySelector("[class*=thinking],[data-testid*=thinking]");
            const hasError = text.includes("Something went wrong") || text.includes("error generating");
            const url = location.href;
            const lastUserMsg = userMsgs.length ? userMsgs[userMsgs.length-1].innerText?.substring(0,100) : "";
            return JSON.stringify({count, text: text.substring(0, 50000), stopBtn, streaming, thinking, hasError, url, userCount: userMsgs.length, lastUserMsg});
        ''')
        try:
            return json.loads(state or '{}')
        except:
            return None

    def wait_response(self, user_msg, max_wait=90):
        """Step 4: Smart wait with granular state detection.

        States: send_failed → thinking → generating → complete | hung | error
        Quick responses (<15s): returned immediately.
        Slow responses: returns status with state so agent can poll intelligently.
        """
        d = self.d
        before = getattr(self, '_msg_count_before', 0)
        last_text_before = getattr(self, '_last_text_before', '')
        send_verified = getattr(self, '_send_verified', False)

        t0 = time.time()
        last_chars = 0  # track progress to detect hung state
        no_progress_count = 0  # consecutive checks with no new chars

        # If send wasn't verified, report immediately
        if not send_verified:
            return error_response('send_failed', 'Message may not have been sent',
                                  suggestion='ChatGPT input may have been blocked. Try again.')

        # Poll for up to 30s (60 × 0.5s) to catch most responses
        log(f'{self.platform}: waiting for response (before={before} msgs)')
        for i in range(60):
            time.sleep(0.5)
            s = self.check_response()
            if not s: continue

            count = s.get('count', 0)
            text = s.get('text', '')
            stop_btn = s.get('stopBtn', False)
            streaming = s.get('streaming', False)
            has_error = s.get('hasError', False)
            chars = len(text)

            new_msg = count > before or (count == before and count > 0 and text != last_text_before and chars > 5)

            # Error state
            if has_error:
                return error_response('gpt_error', 'ChatGPT returned an error', suggestion='Try again')

            # Complete: new message, not streaming, has content
            if new_msg and not stop_btn and not streaming and chars > 2:
                url = s.get('url', '')
                if '/c/' in url: self.conv_url = url
                log(f'{self.platform}: complete at {time.time()-t0:.1f}s ({chars} chars)')
                return save(text, self.platform)

            # Generating: new text appearing
            if new_msg and chars > last_chars:
                no_progress_count = 0
                last_chars = chars
                # If we have substantial content and it's been >15s, return status
                if time.time() - t0 > 15 and chars > 50:
                    log(f'{self.platform}: generating at {time.time()-t0:.1f}s ({chars} chars)')
                    return json.dumps({'status': 'generating', 'chars_so_far': chars,
                                       'suggestion': 'Response is being generated. Use action=read_last when ready, or action=is_streaming to check.'})
                continue

            # Thinking: stop button visible but no text yet
            if stop_btn and chars == 0:
                no_progress_count += 1
                # Normal for complex prompts: wait up to 30s of "thinking"
                if no_progress_count > 40:  # 20s with no chars at all
                    log(f'{self.platform}: possible hung state ({no_progress_count} checks, 0 chars)')
                    return json.dumps({'status': 'thinking', 'chars_so_far': 0, 'elapsed_s': round(time.time()-t0, 1),
                                       'suggestion': 'ChatGPT is still thinking (complex prompt). Use action=is_streaming to check, or action=read_last when ready.'})
                if i % 10 == 9:
                    log(f'{self.platform}: thinking... ({time.time()-t0:.0f}s, 0 chars)')
                continue

            # Streaming but no new chars
            if stop_btn and chars > 0 and chars == last_chars:
                no_progress_count += 1
                if no_progress_count > 20:  # 10s no progress while streaming
                    log(f'{self.platform}: stalled at {chars} chars for {no_progress_count} checks')
                    return json.dumps({'status': 'generating', 'chars_so_far': chars,
                                       'suggestion': 'Response may be stalled. Use action=read_last to get partial content.'})

        # 30s timeout — report final state
        s = self.check_response()
        if s:
            text = s.get('text', '')
            stop_btn = s.get('stopBtn', False)
            chars = len(text)
            count = s.get('count', 0)
            new_msg = count > before or chars > 5

            if new_msg and chars > 2 and not stop_btn:
                url = s.get('url', '')
                if '/c/' in url: self.conv_url = url
                return save(text, self.platform)

            if stop_btn or chars > 0:
                state = 'generating' if chars > 0 else 'thinking'
                return json.dumps({'status': state, 'chars_so_far': chars, 'elapsed_s': round(time.time()-t0, 1),
                                   'suggestion': 'Use action=read_last to get the response when ready, or action=is_streaming to check.'})

        return error_response('no_response', 'No response after 30s', suggestion='ChatGPT may be overloaded or message was not sent.')

    def run(self, msg, wait=True):
        """Full pipeline: ensure → verify → send → wait."""
        for attempt in range(self.max_retries + 1):
            try:
                if not self.ensure():
                    return self.last_error or error_response('platform_unavailable', 'Could not open chat platform')
                if not self.verify_ready():
                    if attempt < self.max_retries:
                        log(f'{self.platform}: not ready, retry {attempt+1}')
                        continue
                    return f'{self.platform}: input not found after retries'
                if not self.send(msg):
                    continue
                if not wait: return 'Sent.'
                return self.wait_response(msg)
            except Exception as e:
                log(f'{self.platform}: pipeline error: {e}')
                if attempt < self.max_retries:
                    time.sleep(1)
                else:
                    return f'Error after {self.max_retries+1} attempts: {e}'
        return 'No response'


# Chat instances
_gpt = ChatPipeline('gpt', 'https://chatgpt.com')
_grok = ChatPipeline('grok', 'https://grok.com')


def chat_via_api(platform, message, api_key, base_url='https://api.openai.com/v1', model='gpt-4o'):
    """Send message via official API. Returns response text or None on failure."""
    try:
        req = urllib.request.Request(
            f'{base_url}/chat/completions',
            data=json.dumps({
                'model': model,
                'messages': [{'role': 'user', 'content': message}],
                'max_tokens': 2000
            }).encode(),
            headers={
                'Content-Type': 'application/json',
                'Authorization': f'Bearer {api_key}'
            }
        )
        resp = urllib.request.urlopen(req, timeout=60)
        result = json.loads(resp.read())
        content = result.get('choices', [{}])[0].get('message', {}).get('content', '')
        if content:
            log(f'{platform}: response via API ({len(content)} chars)')
            return content
    except Exception as e:
        log(f'{platform}: API call failed: {e}')
    return None


@tool_def('gpt', 'Send message to ChatGPT', {'message': 'required', 'action': 'optional: send|read_last|is_streaming|history', 'raw': 'optional bool'}, read_only=False, concurrent=False)
def tool_gpt(args):
    action = args.get('action', 'send')

    if action in ('read_last', 'is_streaming', 'history'):
        _gpt.ensure()
        d = _gpt.d
        if action == 'read_last':
            resp = d.js('const m=document.querySelectorAll("[data-message-author-role=assistant]");return m.length?m[m.length-1].innerText:null')
            return save(resp or 'No messages', 'gpt')
        if action == 'is_streaming':
            s = _gpt.check_response()
            if s:
                chars = len(s.get('text', ''))
                stop_btn = s.get('stopBtn', False)
                streaming = s.get('streaming', False)
                if not stop_btn and not streaming:
                    state = 'complete' if chars > 0 else 'idle'
                elif chars == 0:
                    state = 'thinking'
                else:
                    state = 'generating'
                return json.dumps({'state': state, 'streaming': bool(stop_btn), 'chars': chars, 'open': True})
            return json.dumps({'state': 'unknown', 'streaming': False, 'open': False})
        if action == 'history':
            msgs = d.js(f'const m=[];document.querySelectorAll("[data-message-author-role]").forEach(e=>{{const r=e.getAttribute("data-message-author-role"),t=e.innerText?.trim()?.substring(0,300);if(t)m.push({{role:r,text:t}})}});return JSON.stringify(m.slice(-{int(args.get("count",5))}))')
            try: return '\n'.join(f'> {"YOU" if m["role"]=="user" else "GPT"}: {m["text"][:200]}' for m in json.loads(msgs))
            except: return msgs or 'No messages'

    # API mode: use OpenAI API directly if key is available
    if action == 'send' and OPENAI_API_KEY:
        msg = args.get('message', '')
        if not msg: return 'message required'
        result = chat_via_api('gpt', msg, OPENAI_API_KEY)
        if result:
            return save(result, 'gpt')
        log('gpt: API failed, falling back to browser')

    msg = args.get('message', '')
    if not msg: return 'message required'
    return _gpt.run(msg, wait=args.get('wait', True))

@tool_def('grok', 'Send message to Grok', {'message': 'required', 'action': 'optional: send|read_last|is_streaming|history', 'raw': 'optional bool'}, read_only=False, concurrent=False)
def tool_grok(args):
    action = args.get('action', 'send')

    if action in ('read_last', 'is_streaming', 'history'):
        _grok.ensure()
        d = _grok.d
        if action == 'read_last':
            return save(d.js('const s=[".markdown","div.prose","article"];for(const q of s){const e=document.querySelectorAll(q);if(e.length>0)return e[e.length-1].innerText}return null') or 'No messages', 'grok')
        if action == 'is_streaming':
            return json.dumps({'streaming': bool(d.js('return !!document.querySelector("[class*=streaming],[class*=typing]")')), 'open': True})
        if action == 'history':
            return d.js('const m=document.querySelector("main")||document.body;return m.innerText?.substring(0,2000)') or 'No messages'

    # API mode: use xAI API directly if key is available
    if action == 'send' and XAI_API_KEY:
        msg = args.get('message', '')
        if not msg: return 'message required'
        result = chat_via_api('grok', msg, XAI_API_KEY,
                              base_url='https://api.x.ai/v1',
                              model='grok-3')
        if result:
            return save(result, 'grok')
        log('grok: API failed, falling back to browser')

    msg = args.get('message', '')
    if not msg: return 'message required'
    return _grok.run(msg, wait=args.get('wait', True))

@tool_def('js', 'Execute JavaScript in current page', {'code': 'required'}, read_only=True, concurrent=True)
def tool_js(args):
    """Execute arbitrary JavaScript on current page. For debugging and advanced use."""
    code = args.get('code', '')
    if not code: return 'code required'
    result = chrome().js(code)
    if result is None: return '(null)'
    if result == '': return '(empty string)'
    return str(result)[:5000]

@tool_def('status', 'Show browser status (tabs, URLs, PIDs)', {}, read_only=True, concurrent=True)
def tool_status(args):
    tabs = list(_chrome._tabs.keys()) if _chrome else []
    active = _chrome._active if _chrome else None
    url = _chrome.js('return location.href') if _chrome else None
    return json.dumps({'chrome': _chrome is not None, 'tabs': tabs, 'active': active, 'url': url, 'pids': list(_chrome_pids)}, indent=2)

# ── Plugins ──

@tool_def('plugin', 'Run a YAML automation pipeline', {'name': 'required', 'args': 'optional'}, read_only=False, concurrent=False)
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
            return dispatch_tool(tool_name, tool_args)

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
    # Clean up our per-process profile
    import shutil
    ghost_dir = Path.home() / '.neorender' / f'ghost-{os.getpid()}'
    if ghost_dir.exists():
        try: shutil.rmtree(str(ghost_dir))
        except: pass
    log('Cleanup')

atexit.register(cleanup)

def _signal_handler(*a):
    cleanup()
    sys.exit(0)

signal.signal(signal.SIGTERM, _signal_handler)
signal.signal(signal.SIGINT, _signal_handler)

# ── MCP dispatch ──

def dispatch_tool(name, args):
    """Dispatch tool by name with auto-locking and result persistence."""
    t = TOOLS.get(name)
    if not t:
        return f'Unknown tool: {name}'

    # Auto sequential lock for mutating tools
    if not t['concurrent']:
        with _browser_lock:
            result = t['fn'](args)
    else:
        result = t['fn'](args)

    # Auto persist large results
    if t['max_result'] and isinstance(result, str) and len(result) > t['max_result']:
        result = persist_if_large(result, name)

    return result

def get_mcp_tools():
    """Generate MCP tool list from TOOLS registry."""
    result = []
    for name, t in TOOLS.items():
        properties = {}
        required = []
        for param, desc in t['schema'].items():
            properties[param] = {'type': 'string', 'description': desc}
            if 'required' in desc.lower():
                required.append(param)
                properties[param]['description'] = desc.replace('required', '').strip() or f'{param} parameter'
        result.append({
            'name': name,
            'description': t['description'],
            'inputSchema': {
                'type': 'object',
                'properties': properties,
                'required': required,
            }
        })
    return result

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
        respond(id, {"tools": get_mcp_tools()})
    elif method == 'tools/call':
        name = params.get('name', '')
        args = params.get('arguments', {})
        if name in TOOLS:
            try:
                result = dispatch_tool(name, args)
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

HELP_TEXT = """\
NeoBrowser v3.1.0 — AI Browser MCP Server

Usage:
  neo-browser.py              Start MCP server (stdin/stdout JSON-RPC)
  neo-browser.py --help       Show this help
  neo-browser.py --version    Show version
  neo-browser.py doctor       Check all dependencies and environment

MCP Config (Claude Code):
  {"neo-browser": {"command": "npx", "args": ["-y", "neobrowser"]}}

Requires: Python 3.10+, Google Chrome, websockets (pip)
Docs: https://github.com/pitiflautico/neobrowser
"""

def run_doctor():
    import importlib
    import subprocess
    import sqlite3
    import urllib.request

    PASS = '\033[32m✓\033[0m'
    FAIL = '\033[31m✗\033[0m'
    passed = 0
    total = 0

    def check(label, ok, detail=''):
        nonlocal passed, total
        total += 1
        if ok:
            passed += 1
            print(f"  {PASS} {label}")
        else:
            print(f"  {FAIL} {label}{' — ' + detail if detail else ''}")
        return ok

    print("NeoBrowser Doctor")
    print("─" * 40)

    # 1. Python version
    vi = sys.version_info
    py_ok = vi >= (3, 10)
    check(
        f"Python {vi.major}.{vi.minor}.{vi.micro}",
        py_ok,
        "need Python 3.10+ — upgrade at python.org"
    )

    # 2. Chrome binary exists
    chrome_path = Path(CHROME_BIN)
    chrome_found = chrome_path.exists()
    check(
        f"Chrome found at {CHROME_BIN}",
        chrome_found,
        "install Google Chrome from https://google.com/chrome"
    )

    # 3. Chrome launches headless
    chrome_launches = False
    if chrome_found:
        try:
            result = subprocess.run(
                [CHROME_BIN, '--headless=new', '--dump-dom',
                 '--no-sandbox', '--disable-gpu',
                 '--disable-dev-shm-usage', 'about:blank'],
                capture_output=True, text=True, timeout=10
            )
            chrome_launches = result.returncode == 0 or '<html' in result.stdout.lower()
        except Exception:
            chrome_launches = False
    check(
        "Chrome launches headless",
        chrome_launches,
        "Chrome binary found but failed to start — check permissions or re-install"
    )

    # 4. websockets importable
    ws_ok = False
    try:
        importlib.import_module('websockets')
        ws_ok = True
    except ImportError:
        pass
    check("websockets installed", ws_ok, "pip install websockets")

    # 5. Cookie source — real Chrome profile exists with cookies
    real_profile = Path.home() / 'Library' / 'Application Support' / 'Google' / 'Chrome' / PROFILE
    cookie_count = 0
    cookie_ok = False
    if real_profile.exists():
        cookies_db = real_profile / 'Cookies'
        if cookies_db.exists() and cookies_db.stat().st_size > 0:
            try:
                conn = sqlite3.connect(f'file:{cookies_db}?mode=ro&nolock=1', uri=True)
                cookie_count = conn.execute('SELECT COUNT(*) FROM cookies').fetchone()[0]
                conn.close()
                cookie_ok = cookie_count > 0
            except Exception:
                cookie_ok = False
    if cookie_ok:
        print(f"  {PASS} Cookie source: {PROFILE} ({cookie_count} cookies)")
        passed += 1
    else:
        detail = f"profile not found: {real_profile}" if not real_profile.exists() else "Cookies DB empty or unreadable"
        print(f"  {FAIL} Cookie source — {detail}")
    total += 1

    # 6. Ghost profile dir writable
    ghost_dir = Path.home() / '.neorender'
    ghost_ok = False
    try:
        ghost_dir.mkdir(parents=True, exist_ok=True)
        test_file = ghost_dir / '.write_test'
        test_file.touch()
        test_file.unlink()
        ghost_ok = True
    except Exception as e:
        ghost_ok = False
    check(
        f"Ghost profile: ~/.neorender/ writable",
        ghost_ok,
        f"cannot write to {ghost_dir}"
    )

    # 7. Plugins dir exists and has plugins
    plugin_dir = Path.home() / '.neorender' / 'plugins'
    plugin_count = 0
    plugin_dir_ok = False
    if plugin_dir.exists():
        plugin_count = sum(1 for _ in plugin_dir.glob('*.y*ml'))
        plugin_dir_ok = True
    if plugin_dir_ok:
        suffix = f"{plugin_count} found" if plugin_count else "0 found — create plugins in ~/.neorender/plugins/*.yaml"
        print(f"  {'✓' if plugin_count else '!'} Plugins: {suffix} in ~/.neorender/plugins/")
        passed += 1
    else:
        print(f"  {FAIL} Plugin dir ~/.neorender/plugins/ missing — will be created on first run")
    total += 1

    # 8. Network — can reach example.com
    net_ok = False
    try:
        urllib.request.urlopen('http://example.com', timeout=5)
        net_ok = True
    except Exception:
        pass
    check("Network: example.com reachable", net_ok, "no internet access or DNS failure")

    # 9. PyYAML (optional, needed for plugins)
    yaml_ok = False
    try:
        importlib.import_module('yaml')
        yaml_ok = True
    except ImportError:
        pass
    check("PyYAML installed", yaml_ok, "optional — needed for plugins: pip install pyyaml")

    print()
    if passed == total:
        print(f"  {passed}/{total} checks passed — ready to use")
    else:
        print(f"  {passed}/{total} checks passed — fix the items marked ✗ above")

def main():
    args = sys.argv[1:]
    if args and args[0] in ('--help', '-h'):
        print(HELP_TEXT)
        sys.exit(0)
    if args and args[0] in ('--version', '-v'):
        print('neobrowser 3.1.0')
        sys.exit(0)
    if args and args[0] in ('doctor', '--doctor'):
        run_doctor()
        sys.exit(0)

    log(f'NeoBrowser V3 started — {len(TOOLS)} tools, Ghost Chrome headless, CF bypass')
    try:
        for line in sys.stdin:
            line = line.strip()
            if not line: continue
            try: handle(json.loads(line))
            except json.JSONDecodeError: log(f'JSON err: {line[:80]}')
            except Exception as e: log(f'Error: {e}')
    except Exception as e:
        log(f'Fatal: {e}')
    finally:
        cleanup()

if __name__ == '__main__':
    main()
