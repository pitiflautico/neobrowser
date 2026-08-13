# Borrador: deep-dive técnico #1 — cookie decryption cross-platform (usuario publica)

**Dónde**: blog propio / dev.to (segundo artículo, 1-2 semanas después del primero) / Medium. Los deep-dives técnicos traen estrellas de calidad: gente que entiende el problema.
**Título propuesto**: `How to decrypt Chrome cookies on macOS, Linux and Windows — without being evil about it`

---

```markdown
---
title: How to decrypt Chrome cookies on macOS, Linux and Windows — without being evil about it
published: false
tags: rust, security, chrome, mcp
---

If your tool automates a browser, sooner or later you want the user's *real* sessions — nobody wants to re-login to 40 services so an AI can fill a form. Chrome encrypts cookies at rest, and the decryption dance is different on every OS. Here's how it actually works, and the guardrails that make it not-creepy.

## The encryption

Chrome stores cookies in a SQLite database (`Cookies` in the profile dir). Values are encrypted:

- **macOS / Linux**: AES-128-CBC. The key is derived with PBKDF2 (SHA-1, 1003 iterations, 16-byte output) from a password stored in the OS secret store — macOS Keychain ("Chrome Safe Storage") or Linux secret-service/libsecret. Ciphertext is prefixed `v10`.
- **Windows**: AES-256-GCM. The master key lives in `Local State` (JSON), itself encrypted with DPAPI — which only the same Windows user session can unwrap. Ciphertext prefixed `v10`/`v20` (app-bound encryption on newer Chrome makes this harder by design).

In Rust: `aes`, `cbc`, `aes-gcm`, `pbkdf2`, `hmac`, `sha1`, plus `rusqlite` to read the DB. No unsafe, no shelling out.

## The traps

1. **Never open the live DB.** Chrome holds locks and WAL state. Copy the `Cookies` file to a temp dir first, open the copy read-only, delete it after. (And set restrictive permissions on the temp copy — the rows are encrypted, but why leak metadata.)
2. **A wrong key must fail cleanly.** If PBKDF2 gives you garbage, the CBC padding check rejects it — return an error, never half-decrypted garbage.
3. **Chrome updates move the goalposts.** Version prefixes and app-bound encryption change. Isolate the version handling so a new prefix is a one-line fix.

## The guardrails (this is the part that matters)

Reading someone's cookies is one config flag away from being malware, so [NeoBrowser](https://github.com/pitiflautico/neobrowser) treats it like credential handling:

- **Opt-in only** — nothing touches the real profile unless `NEOBROWSER_REAL_PROFILE` is explicitly set.
- **Profile name whitelist-validated** — no path traversal via env var.
- **Identity cookies excluded** — Google/LinkedIn/Microsoft session-identity cookies are filtered out, so the automation profile can't log your real browser out (and can't fully impersonate you at the identity provider level).
- **0600 everywhere** — session snapshots and playbooks are written owner-only.
- **Attach mode never patches** — pointed at a Chrome you already run, it doesn't modify or kill it.

The code is ~300 lines of Rust, MIT licensed: [`rust/src/cookies.rs`](https://github.com/pitiflautico/neobrowser/blob/main/rust/src/cookies.rs). Steal the approach — the guardrails too.
```

## Notas
- Verificado contra el código real (cookies.rs: validación whitelist, exclusión de identity cookies, copia temporal de la DB, 0600). No publicar nada que se desvíe de esto.
- Si Chrome cambia el esquema (app-bound en más plataformas), el artículo envejece — añadir fecha al publicar.
