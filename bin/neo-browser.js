#!/usr/bin/env node
/**
 * neo-browser — MCP server launcher.
 *
 * 1. Resolves the bundled Rust fast-path binary for this platform.
 * 2. Launches the Python MCP server (tools/v3/neo-browser.py).
 *
 * Binary resolution order:
 *   a) Bundled binary in bin/ (installed via npm, built by CI)
 *   b) NEOBROWSER_V1_BIN env var (custom path)
 *   c) 'neobrowser' in PATH (npm global fallback)
 */

const { spawn, execSync } = require('child_process');
const path = require('path');
const fs = require('fs');

// ── Resolve bundled Rust binary ──────────────────────────────────────────────

const PLATFORM_MAP = {
  'darwin-arm64':  'neobrowser_rs-darwin-arm64',
  'darwin-x64':    'neobrowser_rs-darwin-x64',
  'linux-x64':     'neobrowser_rs-linux-x64',
  'win32-x64':     'neobrowser_rs-win32-x64.exe',
};

const platformKey = `${process.platform}-${process.arch}`;
const bundledName = PLATFORM_MAP[platformKey];
const bundledPath = bundledName ? path.join(__dirname, bundledName) : null;

let v1Bin = null;
if (bundledPath && fs.existsSync(bundledPath)) {
  v1Bin = bundledPath;
} else if (process.env.NEOBROWSER_V1_BIN) {
  v1Bin = process.env.NEOBROWSER_V1_BIN;
} else {
  try {
    execSync('neobrowser --version', { stdio: 'ignore' });
    v1Bin = 'neobrowser';
  } catch {
    process.stderr.write('[neo] Warning: Rust fast-path binary not found. HTTP fetch will use Chrome fallback.\n');
  }
}

// ── Resolve Python ───────────────────────────────────────────────────────────

const pythonCandidates = ['python3', 'python'];
let python = null;
for (const cmd of pythonCandidates) {
  try {
    execSync(`${cmd} --version`, { stdio: 'ignore' });
    python = cmd;
    break;
  } catch {}
}

if (!python) {
  process.stderr.write('Error: Python 3 is required. Install from https://python.org\n');
  process.exit(1);
}

// ── Check deps ───────────────────────────────────────────────────────────────

try {
  execSync(`${python} -c "import websockets"`, { stdio: 'ignore' });
} catch {
  process.stderr.write('[neo] Installing websockets + pyyaml...\n');
  try {
    execSync(`${python} -m pip install "websockets>=14,<16" pyyaml`, { stdio: 'inherit' });
  } catch {
    process.stderr.write('Error: Could not install deps. Run: pip install websockets pyyaml\n');
    process.exit(1);
  }
}

// ── Launch MCP server ────────────────────────────────────────────────────────

const serverPath = path.join(__dirname, '..', 'tools', 'v3', 'neo-browser.py');
const env = { ...process.env };
if (v1Bin) env.NEOBROWSER_V1_BIN = v1Bin;

const child = spawn(python, [serverPath], { stdio: 'inherit', env });
child.on('exit', (code) => process.exit(code || 0));
child.on('error', (err) => {
  process.stderr.write(`[neo] Failed to start: ${err.message}\n`);
  process.exit(1);
});
