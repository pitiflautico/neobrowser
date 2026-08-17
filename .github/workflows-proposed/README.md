# Proposed workflow changes

These three files are the workflows this branch would install, parked outside
`.github/workflows/` for one mechanical reason: pushing a branch that modifies a workflow
requires an OAuth token with the `workflow` scope, and the account that opened this pull
request does not have write access to the repository. Leaving them here keeps the content
reviewable in the diff instead of dropping it.

To apply, from the repository root:

```bash
mv .github/workflows-proposed/{ci,nightly,release}.yml .github/workflows/
rm -r .github/workflows-proposed
```

## What they add

`ci.yml` gains nine steps, and the four worth arguing for are the ones that catch a class of
mistake this branch made and had to find by hand:

- **README claims match the binary** (`scripts/check-claims.py`) — the tool count, the
  advertised-tool count, binary size, environment-variable coverage and internal links, all
  checked against the built binary. This repository has shipped "43 tools" when there were 67
  and "~4 MB static binary" for a 5.5 MB dynamically-linked one.
- **No orphaned attributes** (`scripts/check-orphan-attrs.py`) — an attribute separated from
  its item still compiles and silently applies to the wrong thing. It cost `ClickOutcome` its
  `#[derive]` and `ChromeProcess` its `Debug`, and broke no test.
- **Docs build without warnings** — `cargo doc --no-deps` under `RUSTDOCFLAGS: -D warnings`.
  A broken intra-doc link is invisible until somebody reads the rendered documentation.
- **cargo audit, cargo deny, gitleaks, CycloneDX SBOM** — supply-chain checks. `cargo deny`
  found a real advisory on its first run (a reachable panic in `rustls-webpki 0.103.10`).

The Linux runner also needs `kernel.apparmor_restrict_unprivileged_userns=0` for Chrome's
sandbox to engage on Ubuntu 23.10+, which is why the first step sets it. Without it the
sandbox tests do not run, and this branch makes launching without a sandbox a refusal.
