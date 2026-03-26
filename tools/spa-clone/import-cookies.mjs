#!/usr/bin/env node
/**
 * Chrome Cookie Importer — extracts and decrypts cookies from Chrome profiles.
 *
 * Usage:
 *   node import-cookies.mjs <domain> [--profile "Profile 24"] [--json] [--all]
 *   node import-cookies.mjs chatgpt.com --profile "Profile 24"
 *   node import-cookies.mjs chatgpt.com --all    (all profiles)
 *   node import-cookies.mjs chatgpt.com --json   (output as JSON for Playwright)
 */

import { execSync } from 'child_process';
import { existsSync, copyFileSync, unlinkSync, writeFileSync } from 'fs';
import { join } from 'path';
import { homedir, tmpdir } from 'os';
import { pbkdf2Sync, createDecipheriv } from 'crypto';

// ── Args ──
const args = process.argv.slice(2);
const domain = args.find(a => !a.startsWith('-'));
if (!domain) {
  console.error('Usage: node import-cookies.mjs <domain> [--profile "Profile 24"] [--json] [--all]');
  process.exit(1);
}
const profileArg = args.includes('--profile') ? args[args.indexOf('--profile') + 1] : null;
const showAll = args.includes('--all');
const jsonOutput = args.includes('--json');
const outputFile = args.includes('--output') ? args[args.indexOf('--output') + 1] : null;

// ── Chrome paths ──
const CHROME_BASE = join(homedir(), 'Library', 'Application Support', 'Google', 'Chrome');

// ── Get encryption key from macOS Keychain ──
function getChromeKey() {
  try {
    const password = execSync(
      'security find-generic-password -s "Chrome Safe Storage" -w',
      { encoding: 'utf8' }
    ).trim();
    // PBKDF2-SHA1: 1003 iterations, salt "saltysalt", 16-byte key
    return pbkdf2Sync(password, 'saltysalt', 1003, 16, 'sha1');
  } catch (e) {
    console.error('Cannot get Chrome Safe Storage key from Keychain.');
    console.error('Make sure Chrome is installed and you have Keychain access.');
    process.exit(1);
  }
}

// ── Decrypt a Chrome cookie value ──
function decryptValue(encBuf, aesKey) {
  if (!encBuf || encBuf.length < 4) return null;

  // v10 prefix = AES-128-CBC encrypted (macOS Chrome)
  // Format: "v10" + [16-byte IV/nonce] + [AES-CBC ciphertext with PKCS7 padding]
  if (encBuf[0] === 0x76 && encBuf[1] === 0x31 && encBuf[2] === 0x30) {
    try {
      const payload = encBuf.slice(3);
      // First 16 bytes after v10 are the IV/nonce prepended by Chrome
      const iv = Buffer.alloc(16, 0x20); // Chrome uses spaces as IV for decryption
      const decipher = createDecipheriv('aes-128-cbc', aesKey, iv);
      decipher.setAutoPadding(false);
      let decrypted = Buffer.concat([decipher.update(payload), decipher.final()]);

      // Remove PKCS7 padding
      const padLen = decrypted[decrypted.length - 1];
      if (padLen > 0 && padLen <= 16) {
        let validPad = true;
        for (let i = decrypted.length - padLen; i < decrypted.length; i++) {
          if (decrypted[i] !== padLen) { validPad = false; break; }
        }
        if (validPad) {
          decrypted = decrypted.slice(0, decrypted.length - padLen);
        }
      }

      // Chrome prepends a 16-byte random nonce to the plaintext before encryption.
      // After decryption, the first 16 bytes of output are this nonce → skip them.
      // But the nonce gets scrambled by CBC — so we need to find where the real value starts.
      // Strategy: scan for the first printable ASCII run of sufficient length.
      const str = decrypted.toString('utf8');

      // Chrome AES-CBC: first 32 bytes are garbled (2 CBC blocks = IV scramble + nonce).
      // The real cookie value starts at byte 32.
      if (decrypted.length > 32) {
        return decrypted.slice(32).toString('utf8').replace(/[\x00-\x08\x0e-\x1f]/g, '');
      }
      // Short values: try to find printable content
      const raw = decrypted.toString('utf8');
      const match = raw.match(/[a-zA-Z0-9%{"\[\/].+/);
      return match ? match[0] : raw;
    } catch {
      return null;
    }
  }

  // v11 prefix = AES-256-GCM (Linux Chrome) — not on macOS
  if (encBuf[0] === 0x76 && encBuf[1] === 0x31 && encBuf[2] === 0x31) {
    // Not implemented for macOS — v11 is Linux-only
    return null;
  }

  // No prefix = plaintext (old Chrome)
  return encBuf.toString('utf8');
}

// ── Extract cookies from a profile ──
function extractFromProfile(profileName, searchDomain, aesKey) {
  const cookieDb = join(CHROME_BASE, profileName, 'Cookies');
  if (!existsSync(cookieDb)) return [];

  // Copy DB to avoid lock conflicts
  const tmpDb = join(tmpdir(), `cookie-import-${Date.now()}-${Math.random().toString(36).slice(2)}.db`);
  try {
    copyFileSync(cookieDb, tmpDb);
  } catch {
    return [];
  }

  const cookies = [];

  try {
    // Query with BLOB output via hex encoding
    const query = `SELECT host_key, name, hex(encrypted_value), path, expires_utc, is_secure, is_httponly, samesite, has_expires FROM cookies WHERE (host_key = '${searchDomain}' OR host_key = '.${searchDomain}' OR host_key LIKE '%.${searchDomain}') ORDER BY LENGTH(encrypted_value) DESC`;
    const result = execSync(`sqlite3 "${tmpDb}" "${query}"`, {
      encoding: 'utf8',
      maxBuffer: 50 * 1024 * 1024,
    });

    for (const line of result.split('\n')) {
      if (!line.trim()) continue;
      const parts = line.split('|');
      if (parts.length < 9) continue;

      const [hostKey, name, hexValue, cookiePath, expiresUtc, isSecure, isHttponly, samesite, hasExpires] = parts;
      if (!name || !hexValue) continue;

      // Decrypt
      const encBuf = Buffer.from(hexValue, 'hex');
      const value = decryptValue(encBuf, aesKey);
      if (!value || value.length === 0) continue;

      // Chrome epoch: microseconds since 1601-01-01
      // Unix epoch: seconds since 1970-01-01
      // Offset: 11644473600 seconds
      let expires = -1;
      if (hasExpires === '1' && expiresUtc && expiresUtc !== '0') {
        const chromeTs = parseInt(expiresUtc);
        if (chromeTs > 0) {
          expires = Math.floor(chromeTs / 1000000) - 11644473600;
          // Skip expired cookies
          if (expires < Math.floor(Date.now() / 1000)) continue;
        }
      }

      // Map samesite: 0=unspecified(-1), 1=None, 2=Lax, 3=Strict
      // Playwright expects: 'Strict' | 'Lax' | 'None'
      let sameSiteStr = 'Lax';
      if (samesite === '1') sameSiteStr = 'None';
      else if (samesite === '2') sameSiteStr = 'Lax';
      else if (samesite === '3') sameSiteStr = 'Strict';

      const cookie = {
        name,
        value,
        domain: hostKey,
        path: cookiePath || '/',
        secure: isSecure === '1',
        httpOnly: isHttponly === '1',
        sameSite: sameSiteStr,
      };
      if (expires > 0) cookie.expires = expires;

      cookies.push(cookie);
    }
  } catch (e) {
    // sqlite3 may fail if DB is locked despite copy
  }

  try { unlinkSync(tmpDb); } catch {}
  return cookies;
}

// ── Deduplicate cookies (keep most recent by longest value) ──
function dedup(cookies) {
  const map = new Map();
  for (const c of cookies) {
    const key = `${c.name}@${c.domain}@${c.path}`;
    const existing = map.get(key);
    if (!existing || c.value.length > existing.value.length) {
      map.set(key, c);
    }
  }
  return Array.from(map.values());
}

// ── Main ──
const aesKey = getChromeKey();

// Determine profiles to scan
let profiles = [];
if (profileArg) {
  profiles = [profileArg];
} else if (showAll) {
  try {
    const localState = JSON.parse(
      execSync(`cat "${join(CHROME_BASE, 'Local State')}"`, { encoding: 'utf8' })
    );
    const infoCache = localState?.profile?.info_cache || {};
    profiles = Object.keys(infoCache);
  } catch {
    // Fallback: scan filesystem
    const { readdirSync } = await import('fs');
    profiles = readdirSync(CHROME_BASE).filter(d =>
      d.startsWith('Profile ') || d === 'Default'
    );
  }
} else {
  // Default: try common profiles
  profiles = ['Default', 'Profile 24', 'Profile 1', 'Profile 2', 'Profile 3'];
}

let allCookies = [];
for (const prof of profiles) {
  const cookies = extractFromProfile(prof, domain, aesKey);
  if (cookies.length > 0) {
    console.error(`[${prof}] ${cookies.length} cookies for *${domain}*`);
    allCookies = allCookies.concat(cookies);
  }
}

allCookies = dedup(allCookies);

if (allCookies.length === 0) {
  console.error(`No cookies found for "${domain}" in ${profiles.length} profiles.`);
  process.exit(1);
}

console.error(`Total: ${allCookies.length} unique cookies for *${domain}*`);

// Output
if (outputFile) {
  writeFileSync(outputFile, JSON.stringify(allCookies, null, 2));
  console.error(`Written to ${outputFile}`);
} else if (jsonOutput) {
  console.log(JSON.stringify(allCookies, null, 2));
} else {
  // Table output
  console.log(`\n${'─'.repeat(80)}`);
  console.log(`Cookies for *${domain}* (${allCookies.length} total)`);
  console.log(`${'─'.repeat(80)}`);
  console.log(`${'Name'.padEnd(35)} ${'Domain'.padEnd(20)} ${'Secure'.padEnd(7)} ${'HttpOnly'.padEnd(9)} Value`);
  console.log(`${'─'.repeat(80)}`);
  for (const c of allCookies) {
    const val = c.value.length > 30 ? c.value.substring(0, 27) + '...' : c.value;
    console.log(`${c.name.padEnd(35)} ${c.domain.padEnd(20)} ${String(c.secure).padEnd(7)} ${String(c.httpOnly).padEnd(9)} ${val}`);
  }
  console.log(`${'─'.repeat(80)}`);
}
