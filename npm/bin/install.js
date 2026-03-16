#!/usr/bin/env node
/**
 * NeoBrowser — postinstall script.
 * Downloads the pre-built binary for the current platform from GitHub Releases.
 */

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

const VERSION = "0.3.0";
const REPO = "pitiflautico/neobrowser";
const BIN_DIR = __dirname;

const PLATFORMS = {
  "darwin-arm64": "neobrowser-aarch64-apple-darwin",
  "darwin-x64": "neobrowser-x86_64-apple-darwin",
  "linux-x64": "neobrowser-x86_64-unknown-linux-gnu",
  "linux-arm64": "neobrowser-aarch64-unknown-linux-gnu",
};

function getPlatformKey() {
  return `${process.platform}-${process.arch}`;
}

function download(url) {
  return new Promise((resolve, reject) => {
    https.get(url, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        return download(res.headers.location).then(resolve, reject);
      }
      if (res.statusCode !== 200) {
        return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
      }
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    }).on("error", reject);
  });
}

async function main() {
  const key = getPlatformKey();
  const asset = PLATFORMS[key];

  if (!asset) {
    console.log(`\n  NeoBrowser: no pre-built binary for ${key}.`);
    console.log("  Build from source: cargo build --release\n");
    // Create a stub that prints instructions
    const stub = `#!/usr/bin/env node
console.error("NeoBrowser: no pre-built binary for ${key}.");
console.error("Build from source: git clone https://github.com/${REPO} && cd neobrowser && cargo build --release");
process.exit(1);
`;
    fs.writeFileSync(path.join(BIN_DIR, "neobrowser_rs"), stub, { mode: 0o755 });
    return;
  }

  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${asset}.tar.gz`;
  console.log(`  NeoBrowser: downloading ${asset}...`);

  try {
    const tarball = await download(url);
    const tmpFile = path.join(BIN_DIR, "neobrowser.tar.gz");
    fs.writeFileSync(tmpFile, tarball);
    execSync(`tar xzf ${tmpFile} -C ${BIN_DIR}`, { stdio: "pipe" });
    fs.unlinkSync(tmpFile);
    fs.chmodSync(path.join(BIN_DIR, "neobrowser_rs"), 0o755);
    console.log("  NeoBrowser: installed successfully.\n");
  } catch (err) {
    console.log(`\n  NeoBrowser: binary download failed (${err.message}).`);
    console.log("  This is expected if release v${VERSION} hasn't been published yet.");
    console.log("  Build from source: cargo build --release\n");
    // Create a fallback that tries cargo
    const stub = `#!/bin/sh
if command -v cargo >/dev/null 2>&1; then
  echo "NeoBrowser: building from source..." >&2
  cd "$(dirname "$0")/.." && cargo build --release && exec target/release/neobrowser_rs "$@"
else
  echo "NeoBrowser: binary not found. Install Rust and run: cargo build --release" >&2
  exit 1
fi
`;
    fs.writeFileSync(path.join(BIN_DIR, "neobrowser_rs"), stub, { mode: 0o755 });
  }
}

main();
