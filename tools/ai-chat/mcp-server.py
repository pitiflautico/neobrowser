#!/usr/bin/env python3
"""
AI Chat MCP Server — persistent Chrome sessions for ChatGPT/Grok.

Maintains a single Chrome instance across calls. Conversations persist.
Uses neomode (headless + 5 patches) for Cloudflare bypass.

MCP tools:
  gpt(message, session?)   — Send to ChatGPT, get response
  grok(message)            — Send to Grok, get response
  status()                 — Show active sessions
"""

import json, sys, os, time, atexit, signal, threading

def log(msg):
    print(f'[ai-chat] {msg}', file=sys.stderr, flush=True)

# ── State ──
drivers = {}       # platform → driver
pages_ready = {}   # platform → bool (already navigated)
our_pids = set()   # PIDs we launched — only kill these

# Async response tracking
pending = {}       # platform → {'status': 'waiting'|'streaming'|'done'|'error', 'response': str, 'started': float}
msg_counter = {}   # platform → int (message count for tracking)

# PID file for tracking our Chrome processes across restarts
PID_FILE = os.path.expanduser('~/.neorender/ai-chat-pids.json')

def save_pids():
    """Persist our PIDs so we can clean up after restart."""
    try:
        os.makedirs(os.path.dirname(PID_FILE), exist_ok=True)
        with open(PID_FILE, 'w') as f:
            json.dump(list(our_pids), f)
    except: pass

def load_and_kill_stale_pids():
    """On startup, kill Chrome processes from previous crashed sessions."""
    try:
        if os.path.exists(PID_FILE):
            with open(PID_FILE) as f:
                old_pids = json.load(f)
            killed = 0
            for pid in old_pids:
                try:
                    os.kill(int(pid), 9)
                    killed += 1
                except (ProcessLookupError, PermissionError):
                    pass
            if killed:
                log(f'Killed {killed} stale process(es) from previous session')
                time.sleep(2)
            os.remove(PID_FILE)
    except: pass

# Clean up any stale PIDs from previous crash
load_and_kill_stale_pids()

NEOMODE_PATCHES = '''
Object.defineProperty(screen, 'width', {get: () => 1920});
Object.defineProperty(screen, 'height', {get: () => 1080});
Object.defineProperty(screen, 'availWidth', {get: () => 1920});
Object.defineProperty(screen, 'availHeight', {get: () => 1055});
Object.defineProperty(window, 'outerHeight', {get: () => 1055});
Object.defineProperty(window, 'innerHeight', {get: () => 968});
'''

CHROME_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36'
PROFILE = os.environ.get('NEOBROWSER_PROFILE', 'Profile 24')


def get_driver(platform):
    """Get or create a persistent Chrome driver for a platform."""
    if platform in drivers:
        try:
            # Check if still alive
            _ = drivers[platform].title
            return drivers[platform]
        except:
            log(f'{platform}: driver dead, recreating')
            del drivers[platform]
            pages_ready.pop(platform, None)

    import undetected_chromedriver as uc

    # Kill only OUR previous zombies, not other tools' Chrome
    kill_our_zombies()

    options = uc.ChromeOptions()
    options.add_argument('--window-size=1920,1080')
    options.add_argument('--no-sandbox')
    options.add_argument('--disable-dev-shm-usage')
    options.add_argument(f'--user-agent={CHROME_UA}')
    options.headless = True

    # No user_data_dir — avoids SingletonLock issues on crash/restart.
    # Cookies imported via import_cookies() on first navigation.
    driver = uc.Chrome(options=options, version_main=146)

    # Track the PIDs we own
    if hasattr(driver, 'browser_pid'):
        our_pids.add(driver.browser_pid)
    # Also track the chromedriver service PID
    if hasattr(driver, 'service') and hasattr(driver.service, 'process'):
        our_pids.add(driver.service.process.pid)

    # Neomode patches
    driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_PATCHES})

    drivers[platform] = driver
    save_pids()
    log(f'{platform}: Chrome started (neomode, pids={our_pids})')
    return driver


def kill_our_zombies():
    """Kill only Chrome processes we previously launched."""
    killed = 0
    dead_pids = set()
    for pid in our_pids:
        try:
            os.kill(pid, 9)
            killed += 1
        except ProcessLookupError:
            dead_pids.add(pid)
        except PermissionError:
            dead_pids.add(pid)
    our_pids.difference_update(dead_pids)
    if killed:
        log(f'Killed {killed} zombie(s)')
        time.sleep(1)


def ensure_page(platform, url):
    """Navigate to URL if not already there."""
    driver = get_driver(platform)
    if pages_ready.get(platform):
        # Already on the page — check URL
        current = driver.current_url
        if url in current or platform in current:
            return driver

    log(f'{platform}: navigating to {url}')

    # Import cookies per platform
    if platform == 'chatgpt':
        import_cookies(driver, 'chatgpt.com')
        import_cookies(driver, 'openai.com')
    elif platform == 'grok':
        # Grok uses X/Twitter auth
        import_cookies(driver, 'x.com')
        import_cookies(driver, 'grok.com')

    driver.get(url)
    time.sleep(8)  # Grok SPAs need more time
    pages_ready[platform] = True
    log(f'{platform}: page ready — {driver.title}')
    return driver


def import_cookies(driver, domain):
    """Import cookies from Chrome profile."""
    script_dir = os.path.dirname(os.path.abspath(__file__))
    import_script = os.path.join(script_dir, '..', 'spa-clone', 'import-cookies.mjs')

    if not os.path.exists(import_script):
        return

    try:
        import subprocess
        result = subprocess.run(
            ['node', import_script, domain, '--profile', PROFILE, '--json'],
            capture_output=True, text=True, timeout=15
        )
        if result.returncode != 0:
            return

        cookies = json.loads(result.stdout)
        # Need to be on the domain first
        try:
            driver.get(f'https://{domain}')
            time.sleep(1)
        except:
            pass

        ok = 0
        for c in cookies:
            try:
                cookie = {'name': c['name'], 'value': c['value']}
                if c.get('domain'): cookie['domain'] = c['domain']
                if c.get('path'): cookie['path'] = c['path']
                if c.get('secure'): cookie['secure'] = c['secure']
                if c.get('http_only'): cookie['httpOnly'] = c['http_only']
                driver.add_cookie(cookie)
                ok += 1
            except:
                pass
        log(f'Imported {ok}/{len(cookies)} cookies for {domain}')
    except Exception as e:
        log(f'Cookie import failed: {e}')


def send_chatgpt(message):
    """Send message to ChatGPT and wait for response."""
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys
    from selenium.webdriver.support.ui import WebDriverWait
    from selenium.webdriver.support import expected_conditions as EC

    driver = ensure_page('chatgpt', 'https://chatgpt.com')

    # Find and fill textarea
    try:
        el = WebDriverWait(driver, 10).until(
            EC.presence_of_element_located((By.ID, 'prompt-textarea'))
        )
    except:
        return f'Error: ChatGPT textarea not found. Page: {driver.title}'

    el.click()
    time.sleep(0.3)
    el.send_keys(message)
    time.sleep(0.5)

    # Send
    try:
        btn = driver.find_element(By.CSS_SELECTOR, '[data-testid="send-button"]')
        btn.click()
    except:
        el.send_keys(Keys.RETURN)

    log('Message sent')

    # Start background thread to detect response
    setup_network_monitor(driver)
    pending['chatgpt'] = {'status': 'waiting', 'response': None, 'started': time.time()}
    t = threading.Thread(target=_bg_wait, args=(driver, 'chatgpt'), daemon=True)
    t.start()

    return None  # Caller handles async


def send_grok(message):
    """Send message to Grok — non-blocking."""
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys

    driver = ensure_page('grok', 'https://grok.com')

    try:
        el = driver.find_element(By.CSS_SELECTOR, 'textarea, [contenteditable="true"], [role="textbox"]')
    except:
        return f'Error: Grok input not found. Page: {driver.title}'

    el.send_keys(message)
    time.sleep(0.3)
    el.send_keys(Keys.RETURN)

    log('Message sent')
    setup_network_monitor(driver)
    pending['grok'] = {'status': 'waiting', 'response': None, 'started': time.time()}
    t = threading.Thread(target=_bg_wait, args=(driver, 'grok'), daemon=True)
    t.start()

    return None


def _bg_wait(driver, platform):
    """Background thread: wait for AI response and store it."""
    try:
        response = wait_for_response(driver, platform)
        pending[platform] = {'status': 'done', 'response': response, 'started': pending[platform]['started']}
        log(f'{platform}: response ready ({len(response)} chars)')
    except Exception as e:
        pending[platform] = {'status': 'error', 'response': str(e), 'started': pending[platform].get('started', 0)}


def _poll_until_done(platform, timeout=300):
    """Block until background thread has the response."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if platform in pending:
            status = pending[platform].get('status')
            if status == 'done':
                return pending[platform].get('response', '')
            elif status == 'error':
                return f"Error: {pending[platform].get('response', 'unknown')}"
        time.sleep(0.5)
    return 'Error: Timeout waiting for response'


# ── Network-based SSE stream detection ──

def setup_network_monitor(driver):
    """Inject JS to detect when SSE/fetch stream completes."""
    try:
        driver.execute_script('''
            // Track active SSE/fetch streams
            if (!window.__neo_streams) {
                window.__neo_streams = { active: 0, completed: 0, lastDone: 0 };

                // Intercept fetch to detect streaming responses
                const origFetch = window.fetch;
                window.fetch = async function(...args) {
                    const resp = await origFetch.apply(this, args);
                    const url = typeof args[0] === 'string' ? args[0] : args[0]?.url || '';

                    // Detect chat/conversation API calls
                    if (url.includes('conversation') || url.includes('chat') ||
                        url.includes('/api/') || url.includes('completions')) {
                        const contentType = resp.headers.get('content-type') || '';
                        if (contentType.includes('event-stream') || contentType.includes('text/event') ||
                            contentType.includes('octet-stream') || contentType.includes('ndjson')) {
                            window.__neo_streams.active++;

                            // Clone and consume the stream to detect completion
                            const cloned = resp.clone();
                            const reader = cloned.body?.getReader();
                            if (reader) {
                                (async () => {
                                    try {
                                        while (true) {
                                            const { done } = await reader.read();
                                            if (done) break;
                                        }
                                    } catch {}
                                    window.__neo_streams.active--;
                                    window.__neo_streams.completed++;
                                    window.__neo_streams.lastDone = Date.now();
                                })();
                            }
                        }
                    }
                    return resp;
                };
            }
            // Reset for this message
            window.__neo_streams.active = 0;
            window.__neo_streams.completed = 0;
            window.__neo_streams.lastDone = 0;
        ''')
    except:
        pass


def is_stream_done(driver):
    """Check if SSE stream has completed via network monitoring."""
    try:
        result = driver.execute_script('''
            if (!window.__neo_streams) return null;
            return {
                active: window.__neo_streams.active,
                completed: window.__neo_streams.completed,
                lastDone: window.__neo_streams.lastDone
            };
        ''')
        if result and result.get('completed', 0) > 0 and result.get('active', 1) == 0:
            return True
    except:
        pass
    return False


# ── Smart response detection (no fixed timeout) ──

RESPONSE_DIR = os.path.expanduser('~/.neorender/ai-chat-responses')
os.makedirs(RESPONSE_DIR, exist_ok=True)

def wait_for_response(driver, platform, max_wait=300):
    """Wait until the AI finishes writing. No fixed timeout — detect completion."""

    # Phase 1: Wait for streaming to START (max 15s)
    started = False
    for i in range(15):
        time.sleep(1)
        if is_streaming(driver, platform):
            started = True
            if platform in pending:
                pending[platform]['status'] = 'streaming'
            log(f'Streaming started ({i+1}s)')
            break
        # Also check if response appeared instantly (short answers)
        resp = extract_last_response(driver, platform)
        if resp and len(resp) > 2:
            log(f'Instant response ({i+1}s)')
            return save_and_return(resp, platform)

    if not started:
        # Maybe response already there (very fast)
        resp = extract_last_response(driver, platform)
        if resp and len(resp) > 2:
            return save_and_return(resp, platform)
        log('No streaming detected after 15s')

    # Phase 2: Wait for completion via 3 signals (any = done):
    #   1. Network: SSE stream closed (most reliable)
    #   2. UI: streaming indicator gone + response stable
    #   3. DOM: response text stopped growing
    prev_len = 0
    stable_count = 0
    for i in range(max_wait):
        time.sleep(1)

        # Signal 1: Network stream completed
        if is_stream_done(driver):
            time.sleep(1)  # Small grace period for DOM update
            resp = extract_last_response(driver, platform)
            if resp:
                log(f'Stream done ({i+1}s, {len(resp)} chars)')
                return save_and_return(resp, platform)

        # Signal 2+3: UI indicator gone + text stable
        if not is_streaming(driver, platform):
            resp = extract_last_response(driver, platform)
            if resp:
                cur_len = len(resp)
                if cur_len == prev_len and cur_len > 0:
                    stable_count += 1
                    if stable_count >= 2:
                        log(f'Response stable ({i+1}s, {cur_len} chars)')
                        return save_and_return(resp, platform)
                else:
                    stable_count = 0
                prev_len = cur_len

        if i > 0 and i % 15 == 0:
            log(f'Still waiting... ({i}s, stream_done={is_stream_done(driver)})')

    # Final attempt
    resp = extract_last_response(driver, platform)
    if resp:
        return save_and_return(resp, platform)
    return 'Error: No response detected'


def is_streaming(driver, platform):
    """Detect if the AI is currently generating."""
    try:
        if platform == 'chatgpt':
            return driver.execute_script('''
                return !!document.querySelector('[data-testid="stop-button"]') ||
                       !!document.querySelector('button[aria-label*="Stop"]');
            ''')
        elif platform == 'grok':
            return driver.execute_script('''
                return !!document.querySelector('[class*="streaming"]') ||
                       !!document.querySelector('[class*="typing"]') ||
                       !!document.querySelector('[class*="loading"]');
            ''')
    except:
        pass
    return False


def extract_last_response(driver, platform):
    """Extract the last AI response from the page."""
    try:
        if platform == 'chatgpt':
            return driver.execute_script('''
                const m = document.querySelectorAll('[data-message-author-role="assistant"]');
                if (m.length) return m[m.length-1].innerText;
                const md = document.querySelectorAll('.markdown');
                if (md.length) return md[md.length-1].innerText;
                return null;
            ''')
        elif platform == 'grok':
            # Grok: last response block after our message
            return driver.execute_script('''
                // All response containers
                const all = document.querySelectorAll('div[class*="response"], div[class*="message-content"], article');
                if (all.length > 0) {
                    const last = all[all.length - 1];
                    const text = last.innerText?.trim();
                    if (text && text.length > 2) return text;
                }
                // Fallback: get all text blocks that look like responses
                const blocks = document.querySelectorAll('div > p, div > ul, div > ol, div > pre');
                if (blocks.length > 3) {
                    // Last substantial text block
                    for (let i = blocks.length - 1; i >= 0; i--) {
                        const t = blocks[i].innerText?.trim();
                        if (t && t.length > 20) return t;
                    }
                }
                return null;
            ''')
    except:
        pass
    return None


def save_and_return(response, platform):
    """Save long responses to .md file, return summary to context."""
    MAX_INLINE = 500  # Chars to return directly

    # Always save full response to file
    ts = time.strftime('%Y%m%d-%H%M%S')
    filename = f'{platform}-{ts}.md'
    filepath = os.path.join(RESPONSE_DIR, filename)

    with open(filepath, 'w') as f:
        f.write(f'# {platform.upper()} Response — {ts}\n\n')
        f.write(response)

    log(f'Saved {len(response)} chars to {filepath}')

    if len(response) <= MAX_INLINE:
        return response
    else:
        # Return summary + file path
        preview = response[:MAX_INLINE].rsplit(' ', 1)[0]  # Break at word boundary
        return f'{preview}...\n\n[Full response: {len(response)} chars saved to {filepath}]'


def cleanup():
    """Kill only OUR Chrome processes on exit."""
    for name, driver in drivers.items():
        try:
            driver.quit()
        except:
            pass
    kill_our_zombies()
    log('Cleanup done')

atexit.register(cleanup)
signal.signal(signal.SIGTERM, lambda *a: (cleanup(), sys.exit(0)))


# ── MCP Protocol ──

TOOLS = [
    {
        "name": "gpt",
        "description": "Send a message to ChatGPT. Returns immediately with send confirmation. Use ai_status to check for response. Conversation persists.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Message to send"},
                "raw": {"type": "boolean", "default": False, "description": "If true, don't add AI-to-AI prefix"},
                "wait": {"type": "boolean", "default": True, "description": "If true (default), wait for response. If false, return immediately."},
            },
            "required": ["message"]
        }
    },
    {
        "name": "grok",
        "description": "Send a message to Grok. Returns immediately with send confirmation. Use ai_status to check for response.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Message to send"},
                "wait": {"type": "boolean", "default": True, "description": "Wait for response (default true)"},
            },
            "required": ["message"]
        }
    },
    {
        "name": "ai_status",
        "description": "Check AI chat status. Shows if response is ready, streaming, or waiting. If ready, returns the response.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "platform": {"type": "string", "enum": ["chatgpt", "grok", "all"], "default": "all"},
            }
        }
    }
]


def respond(id, result):
    msg = json.dumps({"jsonrpc": "2.0", "id": id, "result": result})
    sys.stdout.write(msg + '\n')
    sys.stdout.flush()


def respond_error(id, code, message):
    msg = json.dumps({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
    sys.stdout.write(msg + '\n')
    sys.stdout.flush()


def handle(req):
    method = req.get('method', '')
    params = req.get('params', {})
    id = req.get('id')

    if method == 'initialize':
        respond(id, {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "ai-chat", "version": "3.0.0"}
        })

    elif method == 'tools/list':
        respond(id, {"tools": TOOLS})

    elif method == 'tools/call':
        name = params.get('name', '')
        args = params.get('arguments', {})

        if name == 'gpt':
            try:
                msg = args.get('message', '')
                wait = args.get('wait', True)
                send_chatgpt(msg)  # Always non-blocking internally

                if wait:
                    # Block until response ready (default behavior)
                    response = _poll_until_done('chatgpt', timeout=300)
                    respond(id, {"content": [{"type": "text", "text": response}]})
                else:
                    respond(id, {"content": [{"type": "text", "text": "Message sent to ChatGPT. Use ai_status to check for response."}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'grok':
            try:
                msg = args.get('message', '')
                wait = args.get('wait', True)
                send_grok(msg)

                if wait:
                    response = _poll_until_done('grok', timeout=300)
                    respond(id, {"content": [{"type": "text", "text": response}]})
                else:
                    respond(id, {"content": [{"type": "text", "text": "Message sent to Grok. Use ai_status to check for response."}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'ai_status':
            plat = args.get('platform', 'all')
            platforms = ['chatgpt', 'grok'] if plat == 'all' else [plat]
            result = {}

            for p in platforms:
                if p in pending:
                    info = pending[p]
                    elapsed = int(time.time() - info.get('started', time.time()))
                    result[p] = {
                        'status': info['status'],
                        'elapsed': f'{elapsed}s',
                    }
                    if info['status'] == 'done':
                        result[p]['response'] = info['response']
                    elif info['status'] == 'streaming':
                        result[p]['preview'] = '(generating...)'
                elif p in drivers:
                    result[p] = {'status': 'idle', 'ready': True}
                else:
                    result[p] = {'status': 'not started'}

            # If single platform and done, return response directly
            if plat != 'all' and plat in result and result[plat].get('status') == 'done':
                resp = result[plat].get('response', '')
                respond(id, {"content": [{"type": "text", "text": resp}]})
            else:
                respond(id, {"content": [{"type": "text", "text": json.dumps(result, indent=2)}]})

        else:
            respond_error(id, -32601, f"Unknown tool: {name}")

    elif method == 'notifications/initialized':
        pass  # Ignore

    elif id is not None:
        respond_error(id, -32601, f"Unknown method: {method}")


# ── Main loop ──
log('MCP server started (neomode persistent sessions)')

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
        handle(req)
    except json.JSONDecodeError:
        log(f'JSON parse error: {line[:100]}')
    except Exception as e:
        log(f'Error: {e}')
