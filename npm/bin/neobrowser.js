#!/usr/bin/env node
/**
 * NeoBrowser — npm CLI wrapper.
 * Passes all arguments to the Rust binary.
 *
 * Usage:
 *   npx neobrowser mcp          # Start MCP server
 *   npx neobrowser see <url>    # Quick page view
 *   npx neobrowser setup        # Interactive setup
 */

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

const binDir = __dirname;
const binaryName = process.platform === "win32" ? "neobrowser_rs.exe" : "neobrowser_rs";
const binaryPath = path.join(binDir, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error("NeoBrowser binary not found. Run: npm install neobrowser");
  process.exit(1);
}

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
});

child.on("exit", (code) => process.exit(code || 0));
child.on("error", (err) => {
  console.error(`NeoBrowser error: ${err.message}`);
  process.exit(1);
});
