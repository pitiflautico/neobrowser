# Borrador: deep-dive técnico #2 — multiplexer CDP en Rust (usuario publica)

**Dónde**: blog propio / dev.to / r/rust. Audiencia distinta a los posts de marketing: esta es credibilidad de ingeniería.
**Título propuesto**: `Multiplexing the Chrome DevTools Protocol in Rust: one reader, zero races`

---

```markdown
---
title: "Multiplexing the Chrome DevTools Protocol in Rust: one reader, zero races"
published: false
tags: rust, async, tokio, chrome
---

The Chrome DevTools Protocol is a single WebSocket per tab that mixes two traffic shapes on the same wire: *responses* (frames with an `id`, answering your commands) and *events* (frames with a `method`, pushed whenever). Every CDP client has to solve the same problem: who owns `recv()`?

Get it wrong and you get the classic bugs: two tasks racing to read the socket, a response consumed by the wrong waiter, events dropped because nobody was listening, or — my favorite — a pending request that hangs until timeout after the socket already died.

## The model that doesn't have these bugs

In [NeoBrowser](https://github.com/pitiflautico/neobrowser)'s Rust core, exactly **one task owns the socket**. Callers never touch it:

- Commands go in through an **mpsc channel**. The connection task assigns ids and writes frames.
- Each caller gets a **oneshot** back. When a frame arrives with an `id`, the task fulfills the matching oneshot.
- Frames with a `method` are **events**, published on a **broadcast channel** — any number of subscribers, none of them blocking the reader.

```
caller ──mpsc──▶ connection task ──▶ WebSocket
caller ◀─oneshot─ (routed by id) ◀──┘
listeners ◀─broadcast── events ◀────┘
```

## Why this kills whole bug classes

**Responses and events never race** — there's one reader, so there's one place where a frame is classified.

**Disconnects are typed, not hung.** When the socket dies, the connection task drains every pending oneshot with a `Closed` error. Callers fail fast with a real reason instead of hitting a 30s timeout and guessing.

**Errors know what failed.** Each pending request keeps its method name, so a protocol error says `CDP error for 'Page.navigate': ...` — not `error for ''`. (We fixed that the hard way; an LLM agent reading your errors deserves to know which command died.)

**Timeouts are per-command and typed.** `CdpError::Timeout { method, timeout }` — you can catch exactly that, log it, retry that one command. The error enum is the API contract.

## The tokio shape

The connection task is one `select!` loop over two branches: the outbound mpsc and the inbound socket stream. No mutex around the socket, no reader/writer split coordination, no "who closes first" dance. The shared state is just the `HashMap<u64, PendingRequest>` behind a lock, and an `AtomicU64` for ids.

The subtle part is the drain-on-exit: when the loop ends for *any* reason (clean close, error, panic upstream), take the pending map and send `Closed` to every waiter. That single invariant — *a oneshot is always resolved, exactly once* — is what makes the rest of the system boring. Boring is the goal.

Full code, MIT: [`rust/src/cdp.rs`](https://github.com/pitiflautico/neobrowser/blob/main/rust/src/cdp.rs) (~500 lines, tests included — the concurrency tests run against a mock WebSocket server).
```

## Notas
- Verificado contra cdp.rs real: mpsc outbound, oneshot por id, broadcast de eventos, drain con Closed, method en errores de protocolo, AtomicU64 ids, tests de concurrencia contra mock.
- Tercer artículo de la serie; publicar después de tech-cookies.md.
