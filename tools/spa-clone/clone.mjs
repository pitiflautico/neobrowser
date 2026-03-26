#!/usr/bin/env node
/**
 * SPA Cloner — captures a fully-rendered SPA with all assets and API mocks.
 *
 * Usage:
 *   node clone.mjs <url> [options]
 *   node clone.mjs pong <url> --message "hello"   (send message to chat SPA)
 *
 * Options:
 *   --output <dir>       Output directory (default: ./cloned-site)
 *   --wait <ms>          Wait for SPA render (default: 5000)
 *   --interact           Scroll to trigger lazy loads
 *   --profile <name>     Chrome profile name (e.g. "Profile 24", "Default")
 *   --user-data-dir <p>  Chrome user data directory
 *   --headed             Show browser window (not headless)
 *   --message <msg>      For pong mode: message to send
 *   -v, --verbose        Verbose logging
 *
 * Output:
 *   <output>/
 *     index.html          — Rendered DOM with rewritten URLs
 *     manifest.json       — Module graph, stats, metadata
 *     assets/             — JS chunks, CSS, fonts, images
 *     api-mocks/          — Captured API responses (JSON)
 *     source/             — Reconstructed source (if sourcemaps available)
 *     screenshot.png      — Full page screenshot
 */

import { chromium } from 'playwright';
import { writeFile, mkdir, access } from 'fs/promises';
import { createHash } from 'crypto';
import { URL } from 'url';
import path from 'path';
import { homedir } from 'os';

// ── Config ──────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const mode = args[0] === 'pong' ? 'pong' : 'clone';
const url = args.find(a => a.startsWith('http'));
if (!url) {
  console.error('Usage: node clone.mjs <url> [--output dir] [--wait ms] [--profile name]');
  console.error('       node clone.mjs pong <url> --message "hello" [--profile name]');
  process.exit(1);
}

const outputDir = args.includes('--output') ? args[args.indexOf('--output') + 1] : './cloned-site';
const waitMs = parseInt(args.includes('--wait') ? args[args.indexOf('--wait') + 1] : '5000') || 5000;
const interact = args.includes('--interact');
const headed = args.includes('--headed');
const verbose = args.includes('--verbose') || args.includes('-v');
const profileName = args.includes('--profile') ? args[args.indexOf('--profile') + 1] : null;
const userDataDir = args.includes('--user-data-dir') ? args[args.indexOf('--user-data-dir') + 1] : null;
const pongMessage = args.includes('--message') ? args[args.indexOf('--message') + 1] : null;

// Extract cookies from Chrome profile for a given domain
async function extractChromeCookies(domain) {
  const { execSync } = await import('child_process');
  const { readFile } = await import('fs/promises');
  const { tmpdir } = await import('os');

  const profile = profileName || 'Default';
  const chromeBase = path.join(homedir(), 'Library', 'Application Support', 'Google', 'Chrome');
  const cookieDb = path.join(chromeBase, profile, 'Cookies');

  try {
    await access(cookieDb);
  } catch {
    console.error(`[spa-clone] No Cookies DB at ${cookieDb}`);
    return [];
  }

  // Copy DB to avoid lock conflicts with running Chrome
  const tmpDb = path.join(tmpdir(), `spa-clone-cookies-${Date.now()}.db`);
  execSync(`cp "${cookieDb}" "${tmpDb}"`, { stdio: 'ignore' });

  // Get Chrome Safe Storage key from Keychain
  let key;
  try {
    key = execSync('security find-generic-password -s "Chrome Safe Storage" -w', { encoding: 'utf8' }).trim();
  } catch {
    console.error(`[spa-clone] Cannot get Chrome Safe Storage key from Keychain`);
    return [];
  }

  // Derive AES key via PBKDF2
  const crypto = await import('crypto');
  const derivedKey = crypto.pbkdf2Sync(key, 'saltysalt', 1003, 16, 'sha1');

  // Query cookies for domain
  let rows;
  try {
    const result = execSync(
      `sqlite3 "${tmpDb}" "SELECT host_key, name, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite FROM cookies WHERE host_key LIKE '%${domain}%'"`,
      { encoding: 'buffer', maxBuffer: 10 * 1024 * 1024 }
    );
    rows = result.toString().split('\n').filter(r => r.trim());
  } catch (e) {
    console.error(`[spa-clone] SQLite query failed: ${e.message}`);
    return [];
  }

  // Decrypt each cookie
  const cookies = [];
  for (const row of rows) {
    // sqlite3 default separator is |
    // But encrypted_value is binary — we need to query differently
  }

  // Simpler approach: use sqlite3 with hex output for encrypted values
  try {
    const hexResult = execSync(
      `sqlite3 "${tmpDb}" "SELECT host_key, name, hex(encrypted_value), path, expires_utc, is_secure, is_httponly, samesite FROM cookies WHERE host_key LIKE '%${domain}%'"`,
      { encoding: 'utf8', maxBuffer: 10 * 1024 * 1024 }
    );

    for (const line of hexResult.split('\n').filter(l => l.trim())) {
      const parts = line.split('|');
      if (parts.length < 8) continue;
      const [hostKey, name, hexValue, cookiePath, expiresUtc, isSecure, isHttponly, samesite] = parts;

      let value = '';
      try {
        const encBuf = Buffer.from(hexValue, 'hex');
        if (encBuf.length > 3 && encBuf[0] === 0x76 && encBuf[1] === 0x31 && encBuf[2] === 0x30) {
          // v10 prefix — AES-128-CBC decryption
          const iv = Buffer.alloc(16, 0x20);
          const decipher = crypto.createDecipheriv('aes-128-cbc', derivedKey, iv);
          decipher.setAutoPadding(true);
          const ciphertext = encBuf.slice(3);
          value = decipher.update(ciphertext, undefined, 'utf8') + decipher.final('utf8');
        } else {
          value = encBuf.toString('utf8');
        }
      } catch {
        continue; // Skip undecryptable cookies
      }

      if (value && name) {
        // Chrome epoch: microseconds since 1601-01-01
        // Unix epoch: seconds since 1970-01-01
        // Diff: 11644473600 seconds
        let expires = -1;
        if (expiresUtc && expiresUtc !== '0') {
          const chromeEpoch = parseInt(expiresUtc);
          if (chromeEpoch > 0) {
            expires = Math.floor(chromeEpoch / 1000000) - 11644473600;
          }
        }

        const cookie = {
          name,
          value,
          domain: hostKey, // Keep original (with . prefix for subdomain cookies)
          path: cookiePath || '/',
          secure: isSecure === '1',
          httpOnly: isHttponly === '1',
          sameSite: samesite === '1' ? 'Lax' : samesite === '2' ? 'None' : 'Lax',
        };
        if (expires > 0) cookie.expires = expires;
        cookies.push(cookie);
      }
    }
  } catch (e) {
    console.error(`[spa-clone] Cookie decryption failed: ${e.message}`);
  }

  // Cleanup
  try { execSync(`rm "${tmpDb}"`, { stdio: 'ignore' }); } catch {}

  console.error(`[spa-clone] Extracted ${cookies.length} cookies for *${domain}*`);
  return cookies;
}

// Launch Chromium with optional cookies from Chrome profile
async function launchBrowser(targetUrl) {
  const browser = await chromium.launch({ headless: !headed });
  const context = await browser.newContext({
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36',
    viewport: { width: 1920, height: 1080 },
    locale: 'en-US',
  });

  // Import cookies from Chrome profile if --profile is set
  if (profileName) {
    try {
      const domain = new URL(targetUrl).hostname.replace(/^www\./, '');
      console.error(`[spa-clone] Importing cookies for ${domain} from Chrome profile ${profileName}...`);

      // Use the import-cookies script to extract
      const { execSync } = await import('child_process');
      const scriptDir = new URL('.', import.meta.url).pathname;
      const cookiesJson = execSync(
        `node "${scriptDir}import-cookies.mjs" "${domain}" --profile "${profileName}" --json`,
        { encoding: 'utf8', maxBuffer: 10 * 1024 * 1024 }
      );

      const allCookies = JSON.parse(cookiesJson);
      if (allCookies.length > 0) {
        let injected = 0;
        for (const cookie of allCookies) {
          try {
            await context.addCookies([cookie]);
            injected++;
          } catch {}
        }
        console.error(`[spa-clone] Injected ${injected}/${allCookies.length} cookies`);
      }
    } catch (e) {
      console.error(`[spa-clone] Cookie import failed: ${e.message}`);
    }
  }

  const page = await context.newPage();
  return { browser, context, page };
}

// ── Asset tracking ──────────────────────────────────────────────────

const assets = new Map();      // url → { path, contentType, size, body }
const apiMocks = new Map();    // url → { status, headers, body }
const moduleGraph = [];        // { url, type, size, deps }
const consoleMessages = [];
const errors = [];
const networkLog = [];

function log(...args) { if (verbose) console.error('[clone]', ...args); }

function classifyUrl(url, contentType) {
  if (contentType?.includes('json') || url.includes('/api/')) return 'api';
  if (contentType?.includes('javascript') || url.endsWith('.js') || url.endsWith('.mjs')) return 'js';
  if (contentType?.includes('css') || url.endsWith('.css')) return 'css';
  if (contentType?.includes('font') || /\.(woff2?|ttf|otf|eot)/.test(url)) return 'font';
  if (contentType?.includes('image') || /\.(png|jpe?g|gif|svg|webp|ico|avif)/.test(url)) return 'image';
  if (url.endsWith('.map')) return 'sourcemap';
  return 'other';
}

function urlToPath(urlStr, category) {
  try {
    const u = new URL(urlStr);
    let p = u.pathname;
    if (p === '/' || p === '') p = '/index';
    // Remove leading slash, sanitize
    p = p.replace(/^\//, '').replace(/[?#].*$/, '').replace(/[^a-zA-Z0-9._\-\/]/g, '_');
    if (!path.extname(p)) {
      const ext = { js: '.js', css: '.css', json: '.json', image: '.png', font: '.woff2', sourcemap: '.map' }[category] || '.bin';
      p += ext;
    }
    return `assets/${category}/${p}`;
  } catch {
    const hash = createHash('md5').update(urlStr).digest('hex').slice(0, 12);
    return `assets/${category}/${hash}`;
  }
}

// ── Main ────────────────────────────────────────────────────────────

async function clone() {
  const startTime = Date.now();
  console.error(`[spa-clone] Cloning ${url}`);
  console.error(`[spa-clone] Output: ${outputDir}`);
  console.error(`[spa-clone] Wait: ${waitMs}ms`);

  // Create output dirs
  for (const dir of ['assets/js', 'assets/css', 'assets/font', 'assets/image', 'assets/other', 'assets/sourcemap', 'api-mocks', 'source']) {
    await mkdir(path.join(outputDir, dir), { recursive: true });
  }

  const { browser, context, page } = await launchBrowser(url);

  // ── Intercept ALL network requests ──
  let requestCount = 0;
  let totalBytes = 0;

  page.on('response', async (response) => {
    const reqUrl = response.url();
    const status = response.status();
    const contentType = response.headers()['content-type'] || '';
    const category = classifyUrl(reqUrl, contentType);

    requestCount++;
    const entry = { url: reqUrl, status, contentType, category, size: 0, timing: 0 };

    try {
      const body = await response.body();
      entry.size = body.length;
      totalBytes += body.length;

      const localPath = urlToPath(reqUrl, category);

      if (category === 'api') {
        // Save API mock
        const mockPath = `api-mocks/${createHash('md5').update(reqUrl).digest('hex').slice(0, 12)}.json`;
        let bodyStr;
        try { bodyStr = JSON.parse(body.toString()); } catch { bodyStr = body.toString(); }
        apiMocks.set(reqUrl, { status, headers: response.headers(), body: bodyStr });
        await writeFile(path.join(outputDir, mockPath), JSON.stringify({
          url: reqUrl, status, headers: response.headers(), body: bodyStr
        }, null, 2));
        log(`API ${status} ${reqUrl.substring(0, 80)} → ${mockPath}`);
      } else {
        // Save asset
        assets.set(reqUrl, { path: localPath, contentType, size: body.length });
        await writeFile(path.join(outputDir, localPath), body);
        if (category === 'js') {
          moduleGraph.push({ url: reqUrl, type: 'script', size: body.length });
        }
        log(`${category.toUpperCase()} ${status} ${(body.length/1024).toFixed(0)}KB ${reqUrl.substring(0, 80)}`);
      }
    } catch (e) {
      // Some responses (204, redirects) have no body
      log(`SKIP ${status} ${reqUrl.substring(0, 60)} (${e.message})`);
    }

    networkLog.push(entry);
  });

  // ── Capture console ──
  page.on('console', msg => {
    consoleMessages.push({ type: msg.type(), text: msg.text() });
    if (msg.type() === 'error') log(`CONSOLE-ERR: ${msg.text().substring(0, 100)}`);
  });

  page.on('pageerror', err => {
    errors.push({ message: err.message, stack: err.stack });
    log(`PAGE-ERR: ${err.message.substring(0, 100)}`);
  });

  // ── Navigate ──
  console.error(`[spa-clone] Navigating...`);
  try {
    await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 });
  } catch (e) {
    console.error(`[spa-clone] Navigation warning: ${e.message}`);
  }

  // ── Wait for SPA render ──
  console.error(`[spa-clone] Waiting ${waitMs}ms for SPA render...`);
  await page.waitForTimeout(waitMs);

  // ── Interact to trigger lazy loads ──
  if (interact) {
    console.error(`[spa-clone] Interacting (scroll, lazy load triggers)...`);
    // Scroll to bottom to trigger lazy loads
    await page.evaluate(async () => {
      const delay = ms => new Promise(r => setTimeout(r, ms));
      for (let i = 0; i < 5; i++) {
        window.scrollTo(0, document.body.scrollHeight);
        await delay(500);
      }
      window.scrollTo(0, 0);
    });
    await page.waitForTimeout(2000);
  }

  // ── Extract rendered DOM ──
  console.error(`[spa-clone] Extracting rendered DOM...`);
  const renderedHtml = await page.evaluate(() => {
    // Remove script tags to get pure rendered HTML
    const clone = document.documentElement.cloneNode(true);
    // Keep scripts but add data attribute for analysis
    clone.querySelectorAll('script').forEach((s, i) => {
      s.setAttribute('data-clone-index', i);
    });
    return '<!DOCTYPE html>\n' + clone.outerHTML;
  });

  // ── Extract SPA framework info ──
  const spaInfo = await page.evaluate(() => {
    const info = {};
    // React
    const reactRoot = document.querySelector('[data-reactroot], #root, #__next, #app');
    if (reactRoot) {
      const reactKey = Object.keys(reactRoot).find(k => k.startsWith('__reactContainer') || k.startsWith('__reactFiber'));
      info.react = !!reactKey;
      info.reactVersion = window.React?.version || window.__REACT_DEVTOOLS_GLOBAL_HOOK__?.renderers?.values()?.next()?.value?.version || 'unknown';
    }
    // Vue
    info.vue = !!document.querySelector('[data-v-]') || !!window.__VUE__;
    // Next.js
    info.nextjs = !!window.__NEXT_DATA__ || !!document.getElementById('__next');
    // Nuxt
    info.nuxt = !!window.__NUXT__;
    // Angular
    info.angular = !!document.querySelector('[ng-version]') || !!window.ng;
    // Svelte
    info.svelte = !!document.querySelector('[class*="svelte-"]');
    // Visible text length
    info.visibleTextLength = document.body?.innerText?.trim()?.length || 0;
    info.title = document.title;
    info.elementCount = document.querySelectorAll('*').length;
    // Forms
    info.forms = Array.from(document.querySelectorAll('form')).map(f => ({
      action: f.action, method: f.method,
      fields: Array.from(f.querySelectorAll('input,select,textarea')).map(i => ({ name: i.name, type: i.type, placeholder: i.placeholder }))
    }));
    // Links
    info.linkCount = document.querySelectorAll('a[href]').length;
    // Images
    info.imageCount = document.querySelectorAll('img').length;
    return info;
  });

  // ── Check for sourcemaps ──
  console.error(`[spa-clone] Checking for sourcemaps...`);
  const sourcemapUrls = [];
  for (const [assetUrl, asset] of assets) {
    if (asset.contentType?.includes('javascript') || assetUrl.endsWith('.js')) {
      // Check if .map exists
      const mapUrl = assetUrl + '.map';
      try {
        const mapResp = await page.request.get(mapUrl, { timeout: 5000 });
        if (mapResp.ok()) {
          const mapBody = await mapResp.body();
          const mapPath = urlToPath(mapUrl, 'sourcemap');
          await writeFile(path.join(outputDir, mapPath), mapBody);
          sourcemapUrls.push({ js: assetUrl, map: mapUrl, size: mapBody.length });
          log(`SOURCEMAP ${(mapBody.length/1024/1024).toFixed(1)}MB ${mapUrl.substring(0, 60)}`);

          // Try to extract source files from sourcemap
          try {
            const mapJson = JSON.parse(mapBody.toString());
            if (mapJson.sources && mapJson.sourcesContent) {
              let extracted = 0;
              for (let i = 0; i < mapJson.sources.length && i < mapJson.sourcesContent.length; i++) {
                if (mapJson.sourcesContent[i]) {
                  const srcPath = `source/${mapJson.sources[i].replace(/\.\.\//g, '').replace(/[^a-zA-Z0-9._\-\/]/g, '_')}`;
                  const fullPath = path.join(outputDir, srcPath);
                  await mkdir(path.dirname(fullPath), { recursive: true });
                  await writeFile(fullPath, mapJson.sourcesContent[i]);
                  extracted++;
                }
              }
              if (extracted > 0) log(`Extracted ${extracted} source files from ${mapUrl}`);
            }
          } catch {}
        }
      } catch {}
    }
  }

  // ── Screenshot ──
  await page.screenshot({ path: path.join(outputDir, 'screenshot.png'), fullPage: true });
  console.error(`[spa-clone] Screenshot saved`);

  // ── Rewrite URLs in HTML ──
  let rewrittenHtml = renderedHtml;
  for (const [originalUrl, asset] of assets) {
    // Replace absolute URLs with local paths
    rewrittenHtml = rewrittenHtml.replaceAll(originalUrl, asset.path);
  }
  await writeFile(path.join(outputDir, 'index.html'), rewrittenHtml);

  // Also save the raw (unrewritten) HTML
  await writeFile(path.join(outputDir, 'index-raw.html'), renderedHtml);

  // ── Manifest ──
  const elapsed = Date.now() - startTime;
  const manifest = {
    url,
    clonedAt: new Date().toISOString(),
    elapsed_ms: elapsed,
    stats: {
      requests: requestCount,
      totalBytes,
      totalMB: (totalBytes / 1024 / 1024).toFixed(1),
      assets: assets.size,
      apiMocks: apiMocks.size,
      sourcemaps: sourcemapUrls.length,
      jsErrors: errors.length,
      consoleMessages: consoleMessages.length,
    },
    spa: spaInfo,
    moduleGraph,
    sourcemaps: sourcemapUrls.map(s => ({ js: s.js, map: s.map, sizeMB: (s.size/1024/1024).toFixed(1) })),
    apiEndpoints: Array.from(apiMocks.keys()),
    errors,
    networkLog: networkLog.map(e => ({ url: e.url.substring(0, 120), status: e.status, category: e.category, size: e.size })),
  };

  await writeFile(path.join(outputDir, 'manifest.json'), JSON.stringify(manifest, null, 2));

  await browser.close();

  // ── Summary ──
  console.error(`\n${'─'.repeat(60)}`);
  console.error(`SPA Clone Complete: ${url}`);
  console.error(`${'─'.repeat(60)}`);
  console.error(`  Time:        ${(elapsed/1000).toFixed(1)}s`);
  console.error(`  Requests:    ${requestCount}`);
  console.error(`  Total size:  ${(totalBytes/1024/1024).toFixed(1)}MB`);
  console.error(`  Assets:      ${assets.size}`);
  console.error(`  API mocks:   ${apiMocks.size}`);
  console.error(`  Sourcemaps:  ${sourcemapUrls.length}`);
  console.error(`  JS errors:   ${errors.length}`);
  console.error(`  Framework:   ${Object.entries(spaInfo).filter(([k,v]) => v === true).map(([k]) => k).join(', ') || 'none detected'}`);
  console.error(`  Visible text: ${spaInfo.visibleTextLength} chars`);
  console.error(`  Elements:    ${spaInfo.elementCount}`);
  console.error(`  Forms:       ${spaInfo.forms?.length || 0}`);
  console.error(`  Links:       ${spaInfo.linkCount}`);
  console.error(`  Output:      ${outputDir}/`);
  console.error(`${'─'.repeat(60)}`);

  // Print to stdout for piping
  console.log(JSON.stringify(manifest, null, 2));
}

// ── PONG Mode ───────────────────────────────────────────────────────
// Send a message to a chat SPA (ChatGPT, Grok, etc.) and capture the response.

async function pong() {
  if (!pongMessage) {
    console.error('Usage: node clone.mjs pong <url> --message "hello" [--profile name]');
    process.exit(1);
  }

  console.error(`[pong] Target: ${url}`);
  console.error(`[pong] Message: ${pongMessage}`);

  const { browser, context, page } = await launchBrowser(url);

  // Navigate
  console.error(`[pong] Navigating...`);
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 }).catch(() => {});
  await page.waitForTimeout(3000);

  // Screenshot before
  await page.screenshot({ path: '/tmp/pong-before.png' });
  console.error(`[pong] Page loaded, screenshot at /tmp/pong-before.png`);

  // Detect chat platform
  const platform = await page.evaluate(() => {
    if (document.querySelector('[data-testid="send-button"]') || document.querySelector('#prompt-textarea')) return 'chatgpt';
    if (document.querySelector('[placeholder*="want to know"]')) return 'grok';
    if (document.querySelector('[contenteditable="true"]')) return 'generic-chat';
    return 'unknown';
  });
  console.error(`[pong] Platform detected: ${platform}`);

  let response = null;

  if (platform === 'chatgpt') {
    response = await pongChatGPT(page);
  } else if (platform === 'grok') {
    response = await pongGrok(page);
  } else if (platform === 'generic-chat') {
    response = await pongGeneric(page);
  } else {
    // Try to find any input field
    console.error(`[pong] Unknown platform, trying generic approach...`);
    response = await pongGeneric(page);
  }

  await page.screenshot({ path: '/tmp/pong-after.png' });
  console.error(`[pong] Done, screenshot at /tmp/pong-after.png`);

  await browser.close();

  console.log(JSON.stringify({ platform, message: pongMessage, response }, null, 2));
}

async function pongChatGPT(page) {
  console.error(`[pong] ChatGPT: typing message...`);

  // ChatGPT uses ProseMirror — find the contenteditable div
  const editor = page.locator('#prompt-textarea, [contenteditable="true"]').first();
  await editor.waitFor({ timeout: 10000 });

  // Type via ProseMirror dispatch (more reliable than keyboard)
  await page.evaluate((msg) => {
    const el = document.querySelector('#prompt-textarea') || document.querySelector('[contenteditable="true"]');
    if (el) {
      el.focus();
      // Try ProseMirror dispatch first
      const pmView = el.pmViewDesc?.spec?.view || el.closest?.('[class*="ProseMirror"]')?.__view;
      if (pmView) {
        const tr = pmView.state.tr.insertText(msg);
        pmView.dispatch(tr);
      } else {
        // Fallback: set textContent + input event
        el.textContent = msg;
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
    }
  }, pongMessage);

  await page.waitForTimeout(500);

  // Click send button
  const sendBtn = page.locator('[data-testid="send-button"], button[aria-label*="Send"]').first();
  await sendBtn.click({ timeout: 5000 }).catch(() => {
    console.error('[pong] Send button click failed, trying Enter key...');
    return editor.press('Enter');
  });

  console.error(`[pong] Message sent, waiting for response...`);

  // Wait for response — detect streaming via the stop button or typing indicator
  await page.waitForTimeout(3000);

  // Wait for streaming to start (stop button appears)
  let streamStarted = false;
  for (let i = 0; i < 10; i++) {
    streamStarted = await page.evaluate(() => {
      return !!document.querySelector('[data-testid="stop-button"]') ||
             !!document.querySelector('button[aria-label*="Stop"]') ||
             !!document.querySelector('[class*="result-streaming"]') ||
             document.querySelectorAll('[data-message-author-role="assistant"]').length > 0;
    });
    if (streamStarted) break;
    await page.waitForTimeout(1000);
  }

  if (!streamStarted) {
    console.error(`[pong] No streaming detected, waiting extra 5s...`);
    await page.waitForTimeout(5000);
  }

  // Wait for streaming to finish (stop button disappears)
  for (let i = 0; i < 60; i++) {
    const isStreaming = await page.evaluate(() => {
      return !!document.querySelector('[data-testid="stop-button"]') ||
             !!document.querySelector('button[aria-label*="Stop"]') ||
             !!document.querySelector('[class*="result-streaming"]');
    });
    if (!isStreaming && i > 3) break;
    await page.waitForTimeout(1000);
    if (i % 5 === 0) console.error(`[pong] Still streaming... (${i}s)`);
  }

  // Extract the last assistant message — try multiple selectors
  const response = await page.evaluate(() => {
    // Try data attribute first
    const byRole = document.querySelectorAll('[data-message-author-role="assistant"]');
    if (byRole.length > 0) return byRole[byRole.length - 1].innerText;
    // Try markdown container
    const markdown = document.querySelectorAll('.markdown');
    if (markdown.length > 0) return markdown[markdown.length - 1].innerText;
    // Try any message that's not the user's
    const allMsgs = document.querySelectorAll('[class*="agent-turn"], [class*="response"]');
    if (allMsgs.length > 0) return allMsgs[allMsgs.length - 1].innerText;
    // Last resort — get text after the user message
    const body = document.body.innerText;
    const userMsg = body.lastIndexOf('PONG');
    if (userMsg > -1) return body.substring(userMsg + 50, userMsg + 500).trim();
    return null;
  });

  console.error(`[pong] Response: ${(response || 'none').substring(0, 100)}...`);
  return response;
}

async function pongGrok(page) {
  console.error(`[pong] Grok: typing message...`);

  const input = page.locator('[placeholder*="want to know"], textarea, [contenteditable="true"]').first();
  await input.waitFor({ timeout: 10000 });
  await input.fill(pongMessage);
  await page.waitForTimeout(300);

  // Send
  await page.keyboard.press('Enter');
  console.error(`[pong] Message sent, waiting for response...`);

  // Wait for response
  await page.waitForTimeout(3000);
  for (let i = 0; i < 30; i++) {
    const done = await page.evaluate(() => {
      // Check if typing indicator is gone
      const indicators = document.querySelectorAll('[class*="typing"], [class*="loading"], [class*="streaming"]');
      return indicators.length === 0;
    });
    if (done && i > 2) break;
    await page.waitForTimeout(1000);
  }

  const response = await page.evaluate(() => {
    // Get last message block that's not from user
    const blocks = document.querySelectorAll('[class*="message"]');
    const last = blocks[blocks.length - 1];
    return last?.innerText || document.body.innerText.substring(document.body.innerText.length - 500);
  });

  console.error(`[pong] Response: ${(response || 'none').substring(0, 100)}...`);
  return response;
}

async function pongGeneric(page) {
  console.error(`[pong] Generic chat: looking for input...`);

  // Find any text input or contenteditable
  const input = page.locator('textarea, [contenteditable="true"], input[type="text"]').first();
  await input.waitFor({ timeout: 10000 });
  await input.fill(pongMessage);
  await page.waitForTimeout(300);
  await page.keyboard.press('Enter');

  console.error(`[pong] Message sent, waiting 10s for response...`);
  await page.waitForTimeout(10000);

  const response = await page.evaluate(() => document.body.innerText.substring(document.body.innerText.length - 1000));
  console.error(`[pong] Response: ${(response || 'none').substring(0, 100)}...`);
  return response;
}

// ── Entry point ─────────────────────────────────────────────────────

if (mode === 'pong') {
  pong().catch(e => { console.error(`[pong] FATAL: ${e.message}`); process.exit(1); });
} else {
  clone().catch(e => { console.error(`[spa-clone] FATAL: ${e.message}`); process.exit(1); });
}
