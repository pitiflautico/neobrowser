# Contributing to NeoBrowser

## The one rule that matters

**Never report success you did not observe.**

NeoBrowser drives a real browser for an AI that cannot see the screen. A tool that says
"clicked" when the click landed nowhere does not merely fail — it makes the agent build
five more steps on a foundation that was never there, and the failure surfaces far from
its cause.

So every mutating action returns a typed envelope with a `status`, and `uncertain` is a
first-class outcome:

```json
{ "ok": false, "status": "uncertain", "evidence": { "changes": [] },
  "warnings": ["no_observable_change: the mouse events were delivered but nothing changed"] }
```

If a genuinely successful action reports `uncertain`, the bug is in what the state digest
can see (`action::state_js`) — not a reason to loosen the rule. That is how the shadow-DOM
and same-length-text gaps were found and fixed. Do not add a code path that promotes
`uncertain` to `succeeded`.

## Getting set up

```bash
git clone https://github.com/pitiflautico/neobrowser && cd neobrowser/rust
cargo build
cargo test                       # unit + live-Chrome + property/fuzz + embedded-JS
./target/debug/neobrowser doctor --json
```

You need Google Chrome (or Chromium) and Node 20+. Tests that need either **self-skip**
when it is absent rather than failing, so a partial setup still gives you a useful run.

## The gate

Every PR must pass what CI runs:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI additionally runs `cargo audit`, `cargo deny`, secret scanning, and the same suite on
macOS and Windows. A dependency advisory fails the build — that check found a real
`rustls-webpki` vulnerability on its first run.

## Where code goes

`rust/` is the product. [`archive/python-oracle/`](archive/python-oracle/) is the original
Python implementation, archived as a historical reference for the algorithms (cookie
decryption, wall heuristics); **do not add features there**, and do not gate changes on it.

See the module map in [AGENTS.md](AGENTS.md) before adding a file — there is probably
already a home for what you are writing.

## Conventions that are enforced by tests, not review

These will fail your build, so they are worth knowing up front:

| Convention | The test that enforces it |
|---|---|
| `--no-sandbox` is never a default flag | `no_sandbox_is_never_a_default_flag` |
| Every registered tool has a policy class | `every_registered_tool_is_classified` |
| Embedded JavaScript parses | `every_embedded_js_snippet_parses` |
| No `return` at end of line in embedded JS | `no_snippet_returns_across_a_newline` |
| Core toolset names only real tools | `every_core_tool_name_is_registered` |
| `uncertain` never serializes as `ok: true` | `uncertain_never_serializes_as_ok` |

## Beyond CI

`nightly.yml` runs a 12-cell matrix (3 OSes × Chrome stable/beta × isolated/persistent
profile) plus the live bot-detector check that CI skips. Chrome beta failures are advisory
— they are how a breaking upstream change is caught a week early, not a reason to block a
PR.

## Writing tests

Name the invariant, not the mechanism. `a_page_cannot_forge_the_closing_fence` says what
would break if it regressed; `test_fence_2` does not.

New tools get an end-to-end check against a real page, not just a compile. Prefer `data:`
URLs so the test is hermetic. Every bug found during development so far came from driving
a real browser, not from a unit test — the unit tests are how they stay fixed.

## Security

The threat model is not theoretical: this tool points a browser at hostile pages while
holding the user's sessions. Two things to internalise before touching security code:

- **Page content is data, never instructions.** It is fenced and labelled by
  `untrusted::wrap`. A page must not be able to widen a policy, change upload roots, or
  read a file.
- **Fail closed.** An unclassified tool gets the most restrictive class. An unparseable
  URL withholds credentials. A missing keychain refuses to write rather than writing
  plaintext.

Found a vulnerability? Please open a
[private security advisory](https://github.com/pitiflautico/neobrowser/security/advisories/new)
rather than a public issue.

## Claims about the product

The README's comparative claims must be backed by something reproducible. This has been
got wrong before: the benchmark asserted Playwright MCP "exposes no cookie save/restore
tool" and therefore could not persist sessions, which was simply false — it does, via
`--user-data-dir`. If you add a comparison, give the competitor its native capabilities
and measure the outcome, not the presence of a tool with a particular name. See
[bench/README.md](bench/README.md).
