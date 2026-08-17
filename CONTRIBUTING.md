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
can see ([`rust/js/state_digest.js`](rust/js/state_digest.js)) — not a reason to loosen the
rule. That is how the shadow-DOM and same-length-text gaps were found and fixed. Do not add a
code path that promotes `uncertain` to `succeeded`.

The rule is not folklore: it is written down as
[The Verified Action Contract](docs/VERIFIED-ACTIONS.md), and each of its invariants has a
conformance scenario that fails if the invariant is broken. Read it before changing anything
that produces a status. If you believe an invariant is wrong, that is a specification change
with a version bump, not a patch to a call site.

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

CI additionally runs `cargo audit`, `cargo deny`, secret scanning, `cargo doc` with
`RUSTDOCFLAGS=-D warnings`, and the same suite on macOS and Windows. A dependency advisory
fails the build — that check found a real `rustls-webpki` vulnerability on its first run.

## The checks the compiler cannot do

Three scripts in [`scripts/`](scripts/). Each exists because of a defect that `cargo build`
and `cargo test` both accepted, so none of them is redundant with the gate above.

### `scripts/check-orphan-attrs.py` — runs in CI

```bash
python3 scripts/check-orphan-attrs.py
```

Finds an attribute separated from the item it was written for. Rust accepts
`#[derive(Debug)]`, then a doc comment, then an item — and silently applies the derive to the
wrong item, or to nothing. Both programs are valid, so the compiler has no opinion.

This is where splitting a module goes wrong: a boundary drawn at the `pub enum` line leaves
the `#[derive]` above it in the previous chunk. It has already cost `ClickOutcome` its
`#[derive(Debug, Clone, PartialEq, Eq)]` and `ChromeProcess` its `#[derive(Debug)]`. Neither
broke a single test, because nothing in the suite `Debug`-formats those types.

The signature it looks for is an attribute line immediately followed by a doc comment, which
legitimate code never produces — docs go *above* attributes, never between an attribute and
its item.

### `scripts/verify-split.py` — run it during a refactor

```bash
python3 scripts/verify-split.py <original.rs> <new/dir>/*.rs
```

Compares multisets of non-trivial lines before and after a module is broken up, and reports
what was lost or duplicated. It cannot be a CI check, because it needs the pre-split original
— it belongs in the refactor commit itself.

Weaker checks kept missing things: `cargo build` accepts a lost `#[derive]`, and a checker
driven by hand-declared line ranges only covers the ranges someone remembered to declare —
which is exactly where lines went missing. Comparing line multisets does not care how the
split was expressed. It allows the additions a split legitimately produces (`mod`, `use`, a
repeated `impl T {`, a widened visibility) and nothing else.

### `scripts/check-claims.py` — runs in CI

```bash
cd rust && cargo build --release
python3 scripts/check-claims.py
```

Checks the README's factual claims against the built binary: the tool count, the
advertised-tool count, the binary size (with a tolerance, since a rebuild moves it),
that every `NEOBROWSER_*` variable the code reads is documented and no documented one is
unused, and that every internal link resolves.

Documentation drift is found late and by hand otherwise. This README shipped "~4 MB static
binary" for a 5.5 MB dynamically-linked one, and "43 tools" when there were 67. If you add a
claim the script cannot check, verify it by hand and say how in the PR.

## Where code goes

`rust/` is the product. [`archive/python-oracle/`](archive/python-oracle/) is the original
Python implementation, archived as a historical reference for the algorithms (cookie
decryption, wall heuristics); **do not add features there**, and do not gate changes on it.

See the module map in [AGENTS.md](AGENTS.md) before adding a file — there is probably
already a home for what you are writing.

### Module layout and size

Two conventions, both about being able to read a module without a map:

- **A module is under 250 lines, and it is split by responsibility, not by line count.**
  A 1,000-line file is not one thing; it is six things that were never named. Splitting
  `chrome.rs` produced `src/chrome/discover.rs`, `src/chrome/endpoint.rs`,
  `src/chrome/lock.rs`, `src/chrome/process.rs` and `src/chrome/sandbox.rs` — each of those is
  a sentence you can say out loud, which is the actual test. If a split leaves you with a
  `misc.rs`, the boundary is in the wrong place.
- **The modern layout: no `mod.rs`.** A parent module is `src/page.rs` sitting *beside* its
  `src/page/` directory, not `src/page/mod.rs`. Editors then show distinct filenames instead
  of a column of tabs all called `mod.rs`, and the parent sorts next to its children.

The parent file holds what the children share — the public surface, the re-exports, the types
that cross submodule boundaries — and is the place to look first.

When you split an existing module, run `scripts/verify-split.py` and
`scripts/check-orphan-attrs.py` before you push. Both catch things the gate does not.

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

### The conformance suite

```bash
cargo test --test conformance
```

This one is not an ordinary test file. It is the executable form of
[The Verified Action Contract](docs/VERIFIED-ACTIONS.md) — it exercises the scenarios in §6 of
the specification, several of which assert on what must *not* be returned rather than on what
must. A change that makes a tool report `succeeded` where the contract requires `uncertain` is
supposed to fail here even when every other test is green. When you add a scenario, say which
invariant it covers, the way the specification's table does.

It drives a real Chrome and self-skips without one. **A skip is not a pass** — if you are
touching status-producing code, run it somewhere Chrome exists and say in the PR that it ran.
When a scenario needs a new fixture, prefer a `data:` URL for the same reason as above.

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
