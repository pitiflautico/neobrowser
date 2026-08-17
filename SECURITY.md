# Security

## Reporting a vulnerability

Please open a
[private security advisory](https://github.com/pitiflautico/neobrowser/security/advisories/new)
rather than a public issue. Include what you can of: the affected version, the session
mode (`profile_mode` reports it), and a minimal reproduction — a `data:text/html,…` URL is
ideal, because it becomes a permanent regression test.

There is no bug bounty. There is a maintainer who will read it.

## Threat model

Worth stating precisely, because NeoBrowser's position is unusual: it points a browser at
**hostile input by design** while holding **the user's real credentials**. Those two facts
together are the whole security problem.

### What we defend against

| Threat | Defence | Where |
|---|---|---|
| A renderer exploit on any visited page reaching the host | Chrome's sandbox is on by default; launching without it is refused, and refused outright alongside real-profile cookies | `chrome::resolve_sandbox` |
| A page instructing the agent (prompt injection) | Page content is fenced and labelled as untrusted data; injection attempts are named in the envelope; page content cannot widen a policy, change upload roots, or read a file | `untrusted`, `policy` |
| A redirect carrying credentials off-origin | Cookies and non-allowlisted headers are dropped the moment the chain leaves the requested scheme+host+port; leaving is irreversible | `reach::fetch::CredentialScope` |
| SSRF into cloud metadata or a private range | Every hop is re-validated; IPv4-mapped, 6to4 and Teredo disguises are decoded; resolved IPs are pinned against DNS-rebinding | `reach::ssrf` |
| Session material read from disk | Cookies and localStorage sealed with AES-256-GCM under a key from the OS credential store; TTL; overwrite-then-unlink revocation | `vault` |
| An agent reaching somewhere it should not | Central pre-dispatch policy: action classes, domain allow/deny, `developer`/`safe`/`autonomous` profiles | `policy` |
| A web page driving the local HTTP surfaces | Both the bridge and the MCP HTTP transport require a custom header / bearer token, which a cross-origin "simple" request cannot set; the HTTP transport also validates `Origin` by exact host | `bridge`, `http_transport` |
| A path from the network becoming a filesystem path | Session ids and playbook names are reduced to a single validated component | `paths::sanitize_profile_name`, `playbook::sanitize` |
| Secrets in logs and shared artifacts | Redaction at the boundary, before anything enters a trace; the state digest hashes values and excludes password fields entirely | `trace::redact`, `action::state_js` |

### What we explicitly do not defend against

Listed because an unstated limitation reads as a claim.

1. **A same-uid local process.** The MCP stdio transport, the bridge token file and the
   vault key are all reachable by anything already running as this user. That is the same
   trust boundary as the user's shell.
2. **An agent misusing a capability it was granted.** If a tab is shared, or a domain
   allowlisted, actions within it are permitted by definition. The policy engine decides
   *whether*, not *whether it was wise*.
3. **Interactive challenges.** reCAPTCHA, Turnstile and behavioural systems can wall a
   session. NeoBrowser detects and reports them; it does not solve them, and any claim
   that it does would be false.
4. **Terms of service.** Automating a logged-in account may breach the service's terms
   regardless of the mechanism, and enforcement lands on the account. That risk is the
   user's to weigh per site.
5. **Byte-identical reproducible builds.** Not yet; see
   [docs/REPRODUCIBILITY.md](docs/REPRODUCIBILITY.md) for the three specific obstacles.
   Build provenance covers the "is this really from this source" question.

## Scope for an external audit

The PRD requires an independent security audit before 1.0. This is the scope an auditor
should be handed, ordered by how much damage a flaw would do.

### Priority 1 — the credential boundary

- `rust/src/vault.rs` — AES-256-GCM sealing, nonce construction (a repeat under one key is
  catastrophic; see `nonces_do_not_repeat`), key handling across Keychain /
  secret-service / DPAPI, TTL enforcement *before* decryption, revocation.
- `rust/src/cookies.rs` — real-profile cookie decryption. The Windows DPAPI path has
  **never been exercised on a Windows host**; it is written to Chrome's documented format
  and the AES-256-GCM value decryption it feeds is covered by tests. Treat it as unproven.
- `rust/src/sessions.rs` — `write_private`: creation at `0600` via `OpenOptions::mode` plus
  an atomic rename. The specific question: is there any remaining window in which session
  material exists with wider permissions?

### Priority 2 — the network boundary

- `rust/src/reach/ssrf.rs` — the address classifier. Are there disguises beyond
  IPv4-mapped, IPv4-compatible, 6to4 and Teredo that reach a private range?
- `rust/src/reach/fetch.rs` — `CredentialScope`. Can a redirect chain be constructed that
  restores credentials after leaving the origin?
- `rust/src/bridge.rs` and `rust/src/http_transport.rs` — token comparison, `Origin`
  validation, and whether a browser page can reach either surface. An earlier revision of
  the bridge had no authentication at all and a hostile page could have poisoned CDP
  results; the question is whether the current defence is complete.

### Priority 3 — the sandbox and process boundary

- `rust/src/chrome.rs` — `resolve_sandbox` and the detection heuristics. Is there a host
  configuration where the sandbox silently does not engage while detection reports
  `Available`?
- Process reaping: does any path leak a Chrome, and can a profile lock be cleared while a
  live Chrome still holds it?

### Priority 4 — untrusted content handling

- `rust/src/untrusted.rs` — fence forging, and normalisation bypasses beyond the
  zero-width and whitespace cases already covered.
- The JavaScript embedded in the Rust source (33 snippets, syntax-checked by
  `tests/embedded_js.rs`). Injection into a snippet via an unescaped argument is the
  interesting class: arguments go through `serde_json::to_string`, and the question is
  whether every interpolation site does so.

### Out of scope

The archived Python implementation (`archive/python-oracle/`) is not shipped and is not
part of the product.

### What to run

```bash
cd rust
cargo test                       # 284 tests, including property/fuzz suites
cargo audit && cargo deny check  # dependency advisories and licences
./target/release/neobrowser doctor --json
../scripts/verify-release.sh v0.1.7   # independent verification of a published release
```

The property tests in `rust/tests/properties.rs` and the injection suite in
`untrusted::tests` are written as adversarial checks and are a reasonable starting point
for understanding what has already been considered.
