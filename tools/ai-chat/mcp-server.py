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

import json, sys, os, time, atexit, signal

# ── State ──
drivers = {}       # platform → driver
pages_ready = {}   # platform → bool (already navigated)
our_pids = set()   # PIDs we launched — only kill these

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


def log(msg):
    print(f'[ai-chat] {msg}', file=sys.stderr, flush=True)


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

    # Dedicated user-data-dir per platform — isolates from other Chrome-based tools
    profile_dir = os.path.expanduser(f'~/.neorender/ai-chat-profiles/{platform}')
    os.makedirs(profile_dir, exist_ok=True)

    driver = uc.Chrome(options=options, version_main=146, user_data_dir=profile_dir)

    # Track the PIDs we own
    if hasattr(driver, 'browser_pid'):
        our_pids.add(driver.browser_pid)
    # Also track the chromedriver service PID
    if hasattr(driver, 'service') and hasattr(driver.service, 'process'):
        our_pids.add(driver.service.process.pid)

    # Neomode patches
    driver.execute_cdp_cmd('Page.addScriptToEvaluateOnNewDocument', {'source': NEOMODE_PATCHES})

    drivers[platform] = driver
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

    log('Message sent, waiting for response...')

    # Enable network interception to detect SSE stream completion
    setup_network_monitor(driver)

    return wait_for_response(driver, 'chatgpt')


def send_grok(message):
    """Send message to Grok and wait for response."""
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys

    driver = ensure_page('grok', 'https://grok.com')

    # Find input
    try:
        el = driver.find_element(By.CSS_SELECTOR, 'textarea, [contenteditable="true"], [role="textbox"]')
    except:
        return f'Error: Grok input not found. Page: {driver.title}'

    el.send_keys(message)
    time.sleep(0.3)
    el.send_keys(Keys.RETURN)

    log('Message sent, waiting for response...')
    setup_network_monitor(driver)
    return wait_for_response(driver, 'grok')


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
        "description": "Send a message to ChatGPT. Conversation persists across calls — same chat thread.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Message to send"},
                "raw": {"type": "boolean", "default": False, "description": "If true, don't add AI-to-AI prefix"},
            },
            "required": ["message"]
        }
    },
    {
        "name": "grok",
        "description": "Send a message to Grok. Conversation persists across calls.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Message to send"},
            },
            "required": ["message"]
        }
    },
    {
        "name": "ai_status",
        "description": "Check active AI chat sessions.",
        "inputSchema": {"type": "object", "properties": {}}
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
                if not args.get('raw'):
                    msg = msg  # No prefix needed, same conversation
                response = send_chatgpt(msg)
                respond(id, {"content": [{"type": "text", "text": response}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'grok':
            try:
                response = send_grok(args.get('message', ''))
                respond(id, {"content": [{"type": "text", "text": response}]})
            except Exception as e:
                respond(id, {"content": [{"type": "text", "text": f"Error: {e}"}], "isError": True})

        elif name == 'ai_status':
            status = {}
            for p in ['chatgpt', 'grok']:
                if p in drivers:
                    try:
                        title = drivers[p].title
                        status[p] = {"active": True, "title": title, "url": drivers[p].current_url}
                    except:
                        status[p] = {"active": False, "error": "driver dead"}
                else:
                    status[p] = {"active": False}
            respond(id, {"content": [{"type": "text", "text": json.dumps(status, indent=2)}]})

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
