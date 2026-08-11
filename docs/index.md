---
title: NeoBrowser
description: An MCP server that drives real Chrome so AI models can use the web with your real logged-in sessions.
---

# NeoBrowser

**Your AI drives a real Chrome with your real logged-in sessions** — so it lands
already authenticated and presents a genuine browser fingerprint instead of the
headless tells that get bots flagged. An [MCP](https://modelcontextprotocol.io)
server for AI models to use the web the way you do.

> **Honest scope:** NeoBrowser removes the common automation *tells* and reuses your
> logged-in sessions to skip most login walls. It does **not** solve interactive
> challenges — reCAPTCHA, Cloudflare Turnstile, or behavioral systems can still
> challenge you. When that happens it *detects* the wall and tells the model.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh
```

Downloads verify a SHA-256 checksum, and each release binary carries a signed
[build-provenance attestation](https://docs.github.com/actions/security-guides/using-artifact-attestations)
(`gh attestation verify <file> --repo pitiflautico/neobrowser`). Windows binaries and
all checksums are on the [Releases page](https://github.com/pitiflautico/neobrowser/releases/latest).

Register with any MCP client:

```jsonc
{ "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
```

## What it does

- **Real-session browsing** — decrypt + inject cookies from your real Chrome profile (opt-in).
- **Genuine stealth** — passes fingerprint checks like bot.sannysoft with the host's real fingerprint (not spoofed).
- **Bot-wall aware** — detects CAPTCHAs / consent gates / rate-limits / login walls and tells the model.
- **Multi-source search** that routes around walled sources; **real multi-tab**; **43 tools**.

## Links

- [Repository](https://github.com/pitiflautico/neobrowser)
- [Tool reference](https://github.com/pitiflautico/neobrowser/blob/main/docs/TOOLS.md)
- [Architecture & contributing (AGENTS.md)](https://github.com/pitiflautico/neobrowser/blob/main/AGENTS.md)
- [Releases](https://github.com/pitiflautico/neobrowser/releases)

MIT © Daniel Perez Pinazo
