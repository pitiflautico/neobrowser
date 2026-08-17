# The Verified Action Contract

**Version 1.0 — 2026-08-17**

A specification for browser-automation tools that report what happened rather than what
they attempted. Implementation-neutral: NeoBrowser is one implementation, and the
conformance suite in [`rust/tests/conformance.rs`](../rust/tests/conformance.rs) can be
pointed at any tool that speaks MCP.

---

## 1. The problem this exists to solve

Ask a browser-automation tool to click a button and it will usually tell you it succeeded.
What it means is that it dispatched two mouse events at some coordinates. Whether a click
landed, whether the page changed, whether the button was even there — none of that is in
the answer.

For a human driving a browser this gap is invisible, because the human looks at the screen.
For an agent it is the dominant failure mode, and it is worse than an error:

- An error stops the agent. A false success makes it continue.
- It continues into a page it never changed, so every subsequent step reasons from a state
  that does not exist.
- The report says the task was completed.

A tool that says "I could not tell" is more useful than one that guesses right most of the
time, because the caller can retry, escalate, or ask a human. There is no recovery from a
confident wrong answer.

Every non-trivial bug found while building NeoBrowser was an instance of this: a fill that
worked but reported `uncertain` because the digest could not see into a shadow root; a
digest that measured text *length*, so `step 2` → `step 3` looked unchanged; a `return`
swallowed by JavaScript's automatic semicolon insertion, so every observation came back
`undefined` and every action reported `uncertain`. None of them was caught by a unit test.
All of them were caught by driving a real browser and noticing the status was wrong.

That is the argument for a contract: the status is the thing worth testing, and it is
testable independently of how the tool is built.

## 2. Terminology

The key words **MUST**, **MUST NOT**, **SHOULD** and **MAY** are as in RFC 2119.

- **Action** — a single requested operation with an observable intent (click, fill, navigate,
  press, submit). Read-only operations are not actions and are out of scope.
- **Observation** — a value derived from the live page, sufficient to detect that the page
  changed. Its representation is unspecified; its properties are not (§4).
- **Budget** — the wall-clock time an action may consume, decided before it starts.
- **Obstruction** — anything preventing an action that is not the absence of the target:
  an overlay, a disabled control, a target outside the viewport, a covering element.
- **Human gate** — a mechanism a site deploys deliberately to require a human: captcha,
  two-factor prompt, consent wall, payment confirmation.

## 3. Status

Every action **MUST** return exactly one status from this closed set. Implementations
**MUST NOT** add statuses; a new outcome is a new specification version.

| Status | Meaning |
|---|---|
| `succeeded` | The action was performed **and** a change consistent with it was observed. |
| `failed` | The action was attempted and did not take place. |
| `blocked` | An obstruction prevented it. The obstruction **MUST** be named. |
| `needs_human` | A human gate was encountered. The gate **MUST** be named. |
| `requires_confirmation` | Policy requires explicit authorisation before this may proceed. |
| `uncertain` | The action was dispatched; its outcome could not be observed. |

`uncertain` is not a failure. It is the honest answer when the tool did its part and cannot
see the result — and it is the status that makes the rest of the contract meaningful.

### 3.1 Derived success

Where a response carries a boolean success field, that field **MUST** be derived from the
status and **MUST NOT** be independently assignable.

This is the difference between a contract and a convention. If `ok` and `status` are two
fields, some code path will eventually set `ok: true` alongside `status: "uncertain"` — not
maliciously, but because a `?` propagated early or a default was applied. If `ok` is a
function of `status`, that state is unrepresentable and no amount of future carelessness can
produce it.

> NeoBrowser: `ActionStatus` is an enum and `ok()` is a method on it. There is no `ok`
> field to set.

## 4. Invariants

These are normative and each maps to a conformance test in §6.

### I1 — `uncertain` is never promoted

No code path may convert `uncertain` into `succeeded`. Not on retry, not on a second
observation, not on a heuristic, not by a default value.

### I2 — Observation brackets the action

`succeeded` **MUST** require an observation taken before the action, an observation taken
after, and a detected difference between them. An action that reports `succeeded` without
comparing two observations is not conformant, however confident its implementation is.

### I3 — Unobservable is empty, never stale

If the page cannot be observed, the observation **MUST** be empty. It **MUST NOT** be a
cached earlier value. Two empty observations **MUST NOT** compare as changed.

The failure mode this forbids is specific: returning the last known state when the current
state is unavailable makes an action that did nothing look like an action that worked, and
makes a dead page look alive.

### I4 — A dead transport produces errors, never empty successes

When the connection to the browser is gone, operations **MUST** return errors. They
**MUST NOT** return default, empty or zero values that a caller could read as a real result.
`null` handed to a model is indistinguishable from a page that evaluated to nothing.

### I5 — `blocked` names the obstruction

"It failed" is not conformant. "An element with `z-index: 9999` covers the target at
(412, 308)" is. The value of `blocked` is entirely in the diagnosis; without it the status
is a slower `failed`.

### I6 — Cancellation is reported as cancellation

An action interrupted by shutdown **MUST** be reported as cancelled, promptly, and **MUST
NOT** be reported as a timeout (which sends someone debugging a slow page) or as
`succeeded`. Cancellation **MUST** be observed by in-flight waits, not only checked between
actions.

### I7 — Human gates are reported, not defeated

On encountering a human gate an implementation **MUST** return `needs_human` naming the
gate. It **MUST NOT** attempt to solve, bypass or outsource it.

This is a correctness requirement, not only an ethical one: a gate is an explicit statement
by the site operator, and a tool that defeats it cannot be safely run by anyone against
infrastructure they do not own.

### I8 — References are re-resolved at use

An identifier obtained from an observation **MUST** be re-resolved against the live page at
the moment of the action. Implementations **MUST NOT** act on a cached node handle.

A handle invalidated by a re-render between observation and action does not fail — it
silently addresses a different element. Re-resolution costs a round trip and buys the
guarantee that the thing acted on is the thing described.

### I9 — Budgets are bounded and honoured

Every action **MUST** have a finite budget decided before it starts, and **MUST** return a
status when the budget is exhausted rather than continuing. Retries **MUST** consume the same
budget.

### I10 — The status is not derived from the mechanism

Which technique was used (a trusted event, a JavaScript fallback, a keyboard route) **MUST
NOT** affect the status. Only the observed outcome may. A fallback that is easier to verify
**MUST NOT** thereby produce a stronger status than the primary path.

## 5. What this contract does not require

Being explicit about the boundary is part of the specification.

- It does not require any particular observation representation. Hash, tree, screenshot
  diff — any mechanism satisfying §4 conforms.
- It does not require anti-detection behaviour. Trusted events are one way to make an action
  take effect; the contract is about reporting, not about evasion.
- It does not require a specific transport, tool naming, or schema. The conformance suite
  drives MCP because that is what exists; the invariants are transport-independent.
- It does not guarantee an action *will* succeed. It guarantees the report is true.

## 6. Conformance

An implementation conforms to version 1.0 if it passes every scenario below. Each names the
invariant it exercises and the statuses that are acceptable — note that several scenarios
are defined by what **MUST NOT** be returned, because that is where the value is.

| # | Scenario | Required | Forbidden | Invariant |
|---|---|---|---|---|
| C1 | Click a button with a visible effect | `succeeded` | — | I2 |
| C2 | Click a target covered by an overlay | `blocked` + named obstruction | `succeeded` | I5 |
| C3 | Click a button with no handler and no state change | `uncertain` | `succeeded` | I1, I2 |
| C4 | Click a disabled control | `blocked` or `failed` | `succeeded` | I5 |
| C5 | Fill a field inside an open shadow root | `succeeded` | `uncertain` | I2 |
| C6 | Fill a framework-controlled input | `succeeded`, value survives a re-render | — | I10 |
| C7 | Trigger a text change of identical length | `succeeded` | `uncertain` | I2 |
| C8 | Act on a page that never settles | any status within budget | no hang | I9 |
| C9 | Act after the browser process is killed | error | any success | I4 |
| C10 | Interrupt an in-flight wait with shutdown | cancelled, promptly | timeout, `succeeded` | I6 |
| C11 | Encounter a human gate | `needs_human` + named gate | `succeeded`, bypass | I7 |
| C12 | Act on a reference invalidated by a re-render | re-resolve or fail | acting on another element | I8 |
| C13 | Observe an unobservable page twice | empty, no change reported | a fabricated change | I3 |

### 6.1 Running the suite

```
cd rust && cargo test --test conformance
```

The suite drives a real browser. It self-skips when none is present, and a skip is not a
pass — a conformance claim requires the run to have executed.

### 6.2 Claiming conformance

State the version, the commit, and the scenario results. A partial pass is reported as a
partial pass; there is no "mostly conformant". An implementation that reports `succeeded`
where C3 requires `uncertain` is precisely the tool this specification exists to distinguish
itself from.

## 7. Versioning

This document is versioned independently of any implementation. Adding a status, adding an
invariant, or strengthening a scenario is a major version. Clarifying prose is a patch.

## 8. Licence

This specification is published under CC0 1.0 — public domain. Copy it, implement it,
compete with the reference implementation. A contract only becomes a standard if other
people can adopt it without asking.
