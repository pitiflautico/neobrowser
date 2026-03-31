#!/usr/bin/env node

const { spawn, execSync } = require('child_process');
const path = require('path');

// Find Python
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
  console.error('Error: Python 3 is required. Install from https://python.org');
  process.exit(1);
}

// Check websockets
try {
  execSync(`${python} -c "import websockets"`, { stdio: 'ignore' });
} catch {
  console.error('Installing websockets...');
  try {
    execSync(`${python} -m pip install websockets`, { stdio: 'inherit' });
  } catch {
    console.error('Error: Could not install websockets. Run: pip install websockets');
    process.exit(1);
  }
}

// Launch MCP server
const serverPath = path.join(__dirname, '..', 'tools', 'v3', 'neo-browser.py');
const child = spawn(python, [serverPath], {
  stdio: 'inherit',
  env: { ...process.env }
});

child.on('exit', (code) => process.exit(code || 0));
