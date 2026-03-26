#!/usr/bin/env node
/**
 * Stealth Browser — persistent Chrome sessions with anti-detection.
 *
 * Usage:
 *   node browser.mjs open <url>                        Open URL in persistent session
 *   node browser.mjs pong <url> --message "hello"      Send message to chat SPA
 *   node browser.mjs import-cookies <domain>           Import cookies from Chrome
 *   node browser.mjs screenshot <url> [--output f.png] Take screenshot
 *
 * Options:
 *   --session <name>     Session name (default: "default") — reuses profile
 *   --profile <name>     Chrome profile to import cookies from (e.g. "Profile 24")
 *   --headed             Show browser window
 *   --wait <ms>          Wait time after load (default: 5000)
 *   -v, --verbose        Verbose logging
 */

import { chromium } from 'playwright';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'fs';
import { join } from 'path';
import { homedir } from 'os';
import { execSync } from 'child_process';

// ── Args ──
const args = process.argv.slice(2);
const command = args[0] || 'help';
const url = args.find((a, i) => i > 0 && a.startsWith('http'));

function getArg(flag) {
  const i = args.indexOf(flag);
  return i > -1 && i + 1 < args.length ? args[i + 1] : null;
}

const sessionName = getArg('--session') || 'default';
const profileName = getArg('--profile') || null;
const headed = args.includes('--headed');
const verbose = args.includes('-v') || args.includes('--verbose');
const waitMs = parseInt(getArg('--wait') || '5000');
const message = getArg('--message');
const outputFile = getArg('--output');

function log(...a) { if (verbose) console.error('[browser]', ...a); }

// ── Persistent session directory ──
const SESSIONS_DIR = join(homedir(), '.neorender', 'sessions');
const SESSION_DIR = join(SESSIONS_DIR, sessionName);
mkdirSync(SESSION_DIR, { recursive: true });

// ── V1 Stealth JS (canvas, webgl, audio, plugins, timezone, screen, iframe) ──
const STEALTH_JS = `
// WebDriver
Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
delete navigator.__proto__.webdriver;

// Chrome runtime
window.chrome = window.chrome || {};
window.chrome.runtime = window.chrome.runtime || { connect: () => {}, sendMessage: () => {} };
window.chrome.csi = window.chrome.csi || function() { return {}; };
window.chrome.loadTimes = window.chrome.loadTimes || function() { return {}; };

// Permissions
const origQuery = navigator.permissions?.query?.bind(navigator.permissions);
if (origQuery) {
  navigator.permissions.query = (params) => {
    if (params.name === 'notifications') {
      return Promise.resolve({ state: Notification.permission });
    }
    return origQuery(params);
  };
}

// Canvas fingerprint noise
(function() {
  const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
  const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
  function addNoise(canvas) {
    try {
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      const imageData = origGetImageData.call(ctx, 0, 0, canvas.width, canvas.height);
      const d = imageData.data;
      const seed = canvas.width * 7 + canvas.height * 13;
      for (let i = 0; i < d.length; i += 4) {
        d[i] = Math.max(0, Math.min(255, d[i] + ((seed + i * 31) % 5) - 2));
      }
      ctx.putImageData(imageData, 0, 0);
    } catch {}
  }
  HTMLCanvasElement.prototype.toDataURL = function(...a) { addNoise(this); return origToDataURL.apply(this, a); };
})();

// WebGL
(function() {
  const fp = { 37445: 'Google Inc. (Apple)', 37446: 'ANGLE (Apple, Apple M2, OpenGL 4.1)' };
  if (typeof WebGLRenderingContext !== 'undefined') {
    const orig = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(p) { return fp[p] || orig.call(this, p); };
  }
  if (typeof WebGL2RenderingContext !== 'undefined') {
    const orig = WebGL2RenderingContext.prototype.getParameter;
    WebGL2RenderingContext.prototype.getParameter = function(p) { return fp[p] || orig.call(this, p); };
  }
})();

// Plugins
if (navigator.plugins.length === 0) {
  Object.defineProperty(navigator, 'plugins', {
    get: () => {
      const p = [
        {name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'PDF'},
        {name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: ''},
        {name: 'Native Client', filename: 'internal-nacl-plugin', description: ''},
      ];
      p.item = (i) => p[i]; p.namedItem = (n) => p.find(x => x.name === n); p.refresh = () => {};
      return p;
    }
  });
}

// Languages
Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en', 'es'] });

// Hardware concurrency
Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 8 });

// Device memory
Object.defineProperty(navigator, 'deviceMemory', { get: () => 8 });

// Screen
if (screen.width === 0) {
  Object.defineProperty(screen, 'width', {get: () => 1920});
  Object.defineProperty(screen, 'height', {get: () => 1080});
  Object.defineProperty(screen, 'availWidth', {get: () => 1920});
  Object.defineProperty(screen, 'availHeight', {get: () => 1055});
  Object.defineProperty(screen, 'colorDepth', {get: () => 24});
}

// Iframe webdriver leak
(function() {
  try {
    const desc = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
    Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
      get: function() {
        const w = desc.get.call(this);
        if (w) try { Object.defineProperty(w.navigator, 'webdriver', {get: () => undefined}); } catch {}
        return w;
      }
    });
  } catch {}
})();
`;

// ── Cookie import from Chrome profile ──
async function importCookiesFromChrome(context, domain) {
  if (!profileName) return 0;
  try {
    const scriptDir = new URL('.', import.meta.url).pathname;
    const json = execSync(
      `node "${scriptDir}import-cookies.mjs" "${domain}" --profile "${profileName}" --json`,
      { encoding: 'utf8', maxBuffer: 10 * 1024 * 1024 }
    );
    const cookies = JSON.parse(json);
    let ok = 0;
    for (const c of cookies) {
      try { await context.addCookies([c]); ok++; } catch {}
    }
    log(`Imported ${ok}/${cookies.length} cookies for ${domain}`);
    return ok;
  } catch (e) {
    log(`Cookie import failed: ${e.message}`);
    return 0;
  }
}

// ── Launch persistent browser with stealth (NEO mode) ──
// NEO mode: real Chrome window (not headless), offscreen positioning,
// persistent profile, stealth patches. Indistinguishable from real user.
async function launchBrowser() {
  log(`Session: ${sessionName} (${SESSION_DIR})`);

  const launchArgs = [
    '--disable-blink-features=AutomationControlled',  // Hide automation flag
    '--disable-features=IsolateOrigins,site-per-process',
    '--no-sandbox',
    '--disable-dev-shm-usage',
    '--window-size=1920,1080',
  ];

  // NEO mode: never headless. Position offscreen if not --headed
  if (!headed) {
    launchArgs.push('--window-position=-2400,-2400');  // Offscreen
  }

  const context = await chromium.launchPersistentContext(SESSION_DIR, {
    headless: false,  // ALWAYS real Chrome — never headless
    channel: 'chrome',  // Use system Chrome, not Playwright's Chromium
    viewport: { width: 1920, height: 1080 },
    locale: 'en-US',
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36',
    args: launchArgs,
    ignoreDefaultArgs: ['--enable-automation'],  // Don't add automation flags
  });

  // Inject stealth JS before every navigation
  await context.addInitScript(STEALTH_JS);

  log('NEO mode: real Chrome, stealth patches, persistent session');
  return context;
}

// ── Commands ──

async function cmdOpen() {
  if (!url) { console.error('Usage: node browser.mjs open <url>'); process.exit(1); }

  const context = await launchBrowser();
  const page = context.pages()[0] || await context.newPage();

  // Import cookies on first visit
  const domain = new URL(url).hostname.replace(/^www\./, '');
  await importCookiesFromChrome(context, domain);
  // Also import parent domain cookies
  const parts = domain.split('.');
  if (parts.length > 2) await importCookiesFromChrome(context, parts.slice(-2).join('.'));

  console.error(`[browser] Navigating to ${url}...`);
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 }).catch(() => {});
  await page.waitForTimeout(waitMs);

  // Screenshot
  const ssPath = outputFile || '/tmp/browser-screenshot.png';
  await page.screenshot({ path: ssPath, fullPage: true });
  console.error(`[browser] Screenshot: ${ssPath}`);

  // Extract page info
  const info = await page.evaluate(() => ({
    title: document.title,
    url: location.href,
    text: document.body?.innerText?.trim()?.substring(0, 500),
    elements: document.querySelectorAll('*').length,
    forms: document.querySelectorAll('form').length,
    links: document.querySelectorAll('a').length,
    loggedIn: !!(document.querySelector('[class*="avatar"]') || document.querySelector('[class*="user"]') || document.querySelector('[aria-label*="profile"]')),
  }));

  console.log(JSON.stringify(info, null, 2));
  await context.close();
}

async function cmdPong() {
  if (!url || !message) { console.error('Usage: node browser.mjs pong <url> --message "text"'); process.exit(1); }

  const context = await launchBrowser();
  const page = context.pages()[0] || await context.newPage();

  // Import cookies
  const domain = new URL(url).hostname.replace(/^www\./, '');
  await importCookiesFromChrome(context, domain);
  const parts = domain.split('.');
  if (parts.length > 2) await importCookiesFromChrome(context, parts.slice(-2).join('.'));

  console.error(`[pong] Navigating to ${url}...`);
  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 }).catch(() => {});
  await page.waitForTimeout(3000);

  await page.screenshot({ path: '/tmp/pong-before.png' });

  // Detect platform
  const platform = await page.evaluate(() => {
    if (document.querySelector('#prompt-textarea') || document.querySelector('[data-testid="send-button"]')) return 'chatgpt';
    if (document.querySelector('[placeholder*="want to know"]')) return 'grok';
    if (document.querySelector('textarea') || document.querySelector('[contenteditable]')) return 'generic';
    return 'unknown';
  });
  console.error(`[pong] Platform: ${platform}`);

  // Type and send message
  if (platform === 'chatgpt') {
    const textarea = page.locator('#prompt-textarea').first();
    await textarea.waitFor({ timeout: 10000 });
    await textarea.click();
    await page.keyboard.type(message, { delay: 30 });
    await page.waitForTimeout(500);

    // Click send or press Enter
    try {
      await page.locator('[data-testid="send-button"]').click({ timeout: 3000 });
    } catch {
      await page.keyboard.press('Enter');
    }
  } else if (platform === 'grok') {
    const input = page.locator('[placeholder*="want to know"], textarea').first();
    await input.fill(message);
    await page.keyboard.press('Enter');
  } else {
    const input = page.locator('textarea, [contenteditable="true"], input[type="text"]').first();
    await input.waitFor({ timeout: 10000 });
    await input.fill(message);
    await page.keyboard.press('Enter');
  }

  console.error(`[pong] Message sent, waiting for response...`);

  // Wait for streaming to finish
  let response = null;
  for (let i = 0; i < 60; i++) {
    await page.waitForTimeout(1000);

    if (platform === 'chatgpt') {
      // Check if still streaming
      const streaming = await page.evaluate(() =>
        !!document.querySelector('[data-testid="stop-button"]') ||
        !!document.querySelector('button[aria-label*="Stop"]')
      );

      if (!streaming && i > 3) {
        // Extract response
        response = await page.evaluate(() => {
          const msgs = document.querySelectorAll('[data-message-author-role="assistant"]');
          if (msgs.length > 0) return msgs[msgs.length - 1].innerText;
          const md = document.querySelectorAll('.markdown');
          if (md.length > 0) return md[md.length - 1].innerText;
          return null;
        });
        if (response) break;
      }
    } else if (platform === 'grok') {
      const done = await page.evaluate(() => {
        const indicators = document.querySelectorAll('[class*="typing"], [class*="loading"]');
        return indicators.length === 0;
      });
      if (done && i > 3) {
        response = await page.evaluate(() => {
          const msgs = document.querySelectorAll('[class*="message"]');
          return msgs.length > 1 ? msgs[msgs.length - 1].innerText : null;
        });
        if (response) break;
      }
    }

    if (i % 5 === 0) console.error(`[pong] Waiting... (${i}s)`);
  }

  await page.screenshot({ path: '/tmp/pong-after.png' });
  console.error(`[pong] Response: ${(response || 'none').substring(0, 100)}`);

  console.log(JSON.stringify({ platform, message, response }, null, 2));
  await context.close();
}

async function cmdImportCookies() {
  const domain = args[1];
  if (!domain) { console.error('Usage: node browser.mjs import-cookies <domain>'); process.exit(1); }

  const context = await launchBrowser();
  const count = await importCookiesFromChrome(context, domain);
  console.error(`Imported ${count} cookies for ${domain} into session "${sessionName}"`);
  await context.close();
}

async function cmdScreenshot() {
  if (!url) { console.error('Usage: node browser.mjs screenshot <url>'); process.exit(1); }

  const context = await launchBrowser();
  const page = context.pages()[0] || await context.newPage();

  const domain = new URL(url).hostname.replace(/^www\./, '');
  await importCookiesFromChrome(context, domain);

  await page.goto(url, { waitUntil: 'networkidle', timeout: 60000 }).catch(() => {});
  await page.waitForTimeout(waitMs);

  const ssPath = outputFile || '/tmp/screenshot.png';
  await page.screenshot({ path: ssPath, fullPage: true });
  console.error(`Screenshot: ${ssPath}`);
  await context.close();
}

// ── Main ──
switch (command) {
  case 'open': await cmdOpen(); break;
  case 'pong': await cmdPong(); break;
  case 'import-cookies': await cmdImportCookies(); break;
  case 'screenshot': await cmdScreenshot(); break;
  default:
    console.error('Commands: open, pong, import-cookies, screenshot');
    console.error('Example: node browser.mjs pong https://chatgpt.com --message "hello" --profile "Profile 24"');
    process.exit(1);
}
