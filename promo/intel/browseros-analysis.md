# Competitor intel: BrowserOS

**Repo:** https://github.com/browseros-ai/BrowserOS  
**Web:** https://www.browseros.com  
**YC:** S24 (Nikhil Sonti, Nithin Sonti)  
**Stars:** ~12k–13.1k  
**Forks:** ~1k–1.4k  
**License:** AGPL-3.0  
**Stack:** Chromium fork + TypeScript/Go/Rust

---

## Products

BrowserOS ships two products from one monorepo:

| Product | What it is | Target user |
|---|---|---|
| **BrowserOS** | Chromium fork with built-in AI agent in every tab | End users who want a local, private AI browser |
| **BrowserOS neo** (ex BrowserClaw) | Second browser that coding agents control via MCP, using logged-in accounts | Developers using Claude Code, Codex, Cursor, etc. |

Core pitch: *"Two browsers: one for your agents, one for you."*

## Stack

- Chromium fork with ungoogled-chromium privacy patches.
- Agent platform in TypeScript/Go/Rust.
- MCP server HTTP on `127.0.0.1:9239/mcp`.
- 53+ MCP tools.
- Native downloads: `.dmg`, `.exe`, `.AppImage`, `.deb`.

## Pricing

- 100% free and open source (AGPL-3.0).
- User pays only their own API keys / local models.

## Key differences from NeoBrowser

| Dimension | BrowserOS | NeoBrowser |
|---|---|---|
| Architecture | Chromium fork | Drives user's unmodified Google Chrome via CDP |
| Install | New browser binary | Single ~5.5 MB MCP server binary |
| Sessions | Imports Chrome logins into new browser | Uses real Chrome profile live |
| License | AGPL-3.0 | (verify repo LICENSE) |
| MCP tools | 53+ | 67 |
| Sandbox | Depends on their fork | Keeps Chrome sandbox; refuses `--no-sandbox` with real profile |
| DevTools/debugging | "Coming soon" in MCP | Full CDP, network, console, HAR, Web Vitals today |

## Strengths (hard to copy)

1. **Clear dual-product story.** Users instantly get "one browser for me, one for agents."
2. **Strong MCP onboarding.** One-liner Claude/Codex/Cursor setup.
3. **Chromium fork ownership.** Can patch privacy, ads, extensions natively.
4. **YC-backed momentum.** ~13k stars, Product Hunt featured, strong community.
5. **BYOK + local.** No SaaS tax, no data leaving the machine.

## Weaknesses / gaps NeoBrowser can exploit

1. **Must install a new browser.** Huge friction in locked-down corporate environments.
2. **AGPL-3.0.** Scares commercial integrators.
3. **Less emphasis on verification/security.** No clear verified-action contract or policy engine.
4. **DevTools still "coming soon."** NeoBrowser already has full CDP observability.
5. **BrowserOS is a product; NeoBrowser can be infrastructure.** Not tied to replacing the user's browser.

## Recommended differentiation

| Area | Message |
|---|---|
| Zero-install | "Use the Chrome you already have. No new browser to install." |
| Verification | "Every mutating action is checked against real page state." |
| Enterprise license | "No AGPL copyleft risk." |
| DevTools | "Full CDP debugging today: network, console, HAR, Web Vitals." |
| Sandbox | "Keeps Chrome's real sandbox." |
| Vendor lock-in | "Works with any MCP client; not tied to our browser." |

## Conclusion

BrowserOS is a **product competitor** with brand, YC credibility, and a clear narrative. Its strategy is to *replace/duplicate your browser*.

NeoBrowser should not try to be another browser. It should own the position of **reliable, secure, verifiable control layer for the Chrome the user already has**.
