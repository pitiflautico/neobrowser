# NeoRender Protocol (NRP) — v0.1 Draft (Grok-reviewed)

## Architecture: Core Protocol + Agent Helpers

Split into two layers:

### Core Protocol (orthogonal, versionable, stable)
- **Page** — navigation lifecycle
- **Runtime** — JS evaluation with object handles
- **Network** — observation + interception
- **SemanticTree** — DOM-derived navigable tree (NOT AXtree — no spec equivalence)
- **Interact** — low-level input dispatch with typed targets (node_id only)
- **Wait** — condition-based polling with composite signals
- **Session** — lifecycle, capabilities, versioning
- **Observe** — event subscriptions with ordering guarantees
- **Cookies** — cookie management
- **Storage** — localStorage/sessionStorage

### Agent Helpers (high-level, heuristic, may change freely)
- **Form** — extract, fill, validate, submit (composes Core commands)
- **Content** — WOM, semantic text, search (derived from SemanticTree)
- **TargetResolver** — text/role/label → node_id resolution
- **CSRFDetector** — token extraction heuristic

Agent Helpers are NOT part of the wire protocol.

---

## Identity & Epochs

```
session_id:     string  — unique per browser session
page_id:        u64     — monotonic, incremented on navigate()
document_epoch: u64     — incremented on DOM replacement
node_id:        string  — stable within (session_id, page_id, document_epoch)
```

All node_ids scoped to document_epoch. Invalid after navigation or DOM replacement.

---

## Target (typed union — NOT ambiguous string)

```json
{"by": "node_id", "value": "n123"}
{"by": "css", "value": "button[type=submit]"}
{"by": "text", "value": "Submit", "exact": false}
{"by": "label", "value": "Email"}
{"by": "role", "value": "button", "name": "Submit"}
```

TargetResolver (agent-side) resolves Target → node_id.
Core Interact commands ONLY accept node_id.

---

## ActionOutcome (common return type)

```rust
enum ActionOutcome {
    NoEffect,
    DomChanged { mutations: u32 },
    HttpNavigation { url: String, method: String, status: u16 },
    SpaNavigation { url: String },
    FormSubmitted { action: String, method: String },
    ValidationBlocked { invalid_fields: Vec<String> },
    DialogOpened { dialog_type: String },
    DialogClosed,
    FocusMoved { from: Option<String>, to: String },
    CheckboxToggled { checked: bool },
    RadioSelected { value: String },
    Error { message: String },
}
```

---

## Core Domains

### Page
| Command | Params | Returns |
|---------|--------|---------|
| navigate | {url} | {page_id, document_epoch, url, title, status} |
| reload | {} | same |
| back | {} | same or {no_history} |
| forward | {} | same |
| getInfo | {} | {url, title, page_id, document_epoch, ready_state} |
| close | {} | {ok} |

Events: navigated, loadStarted, loadFinished, domContentLoaded

### Runtime
| Command | Params | Returns |
|---------|--------|---------|
| evaluate | {expression, return_by_value?, object_group?} | {result, object_id?, type} |
| callFunction | {object_id, function, arguments} | {result} |
| releaseObject | {object_id} | {ok} |
| getProperties | {object_id} | {properties} |

Events: consoleMessage, exceptionThrown, unhandledRejection

### Network
| Command | Params | Returns |
|---------|--------|---------|
| enable | {} | {ok} |
| getLog | {filter?} | {entries: [RequestEntry]} |
| intercept | {pattern, stage} | {ok} |
| continue | {request_id, headers?} | {ok} |
| fulfill | {request_id, status, headers, body_base64?, truncated?} | {ok} |
| fail | {request_id, reason} | {ok} |

Events: requestStarted, responseReceived, requestCompleted, requestFailed

### SemanticTree
```
SemanticNode {
    node_id, role, name, value?, description?, tag,
    properties: {disabled, required, checked?, selected?, expanded?,
                 focused, editable, readonly, type?, href?, action?, method?},
    actions: ["click","type","select","check","submit","expand","focus"],
    children: [SemanticNode]
}
```

Limitations (documented): no real ARIA computation, no CSS visibility, heuristic roles.

| Command | Params | Returns |
|---------|--------|---------|
| get | {depth?, root_node_id?} | {tree, document_epoch} |
| getNode | {node_id} | {node} |
| find | {role?, name?, text?} | {nodes: []} |
| getFlat | {filter?} | {nodes: []} |

### Interact (node_id only — no resolution)
| Command | Params | Returns |
|---------|--------|---------|
| click | {node_id} | {outcome: ActionOutcome} |
| type | {node_id, text} | {outcome} |
| pressKey | {node_id, key, modifiers?} | {outcome} |
| select | {node_id, value} | {outcome} |
| check | {node_id, checked} | {outcome} |
| focus | {node_id} | {outcome} |
| clear | {node_id} | {outcome} |
| upload | {node_id, files} | {outcome} |

### Wait (composite signals)
| Command | Params | Returns |
|---------|--------|---------|
| forSelector | {css, timeout_ms} | {found, node_id?} |
| forText | {text, css?, timeout_ms} | {found, node_id?} |
| forNavigation | {timeout_ms} | {navigated, url?} |
| forStable | {timeout_ms, signals} | {stable, reason} |
| forHidden | {css, timeout_ms} | {hidden} |

forStable signals: {dom_quiet_ms, network_idle, no_pending_modules}

### Session
| Command | Params | Returns |
|---------|--------|---------|
| create | {config?} | {session_id} |
| destroy | {session_id} | {ok} |
| getCapabilities | {} | {protocol_version, engine_version, domains, features} |
| reset | {what: ["cookies","storage","history","subscriptions","page"]} | {ok} |

### Observe
| Command | Params | Returns |
|---------|--------|---------|
| subscribe | {events, filter?} | {subscription_id} |
| unsubscribe | {subscription_id} | {ok} |
| getBuffer | {subscription_id, since_seq?} | {events, last_seq} |

Contract: monotonic sequence_id, max 1000 buffered, scoped to page_id, at-most-once.

### Cookies / Storage
Separate domains. Clean split. See original PDR for commands.

---

## DOM Change Events (enriched)

```json
{
    "sequence_id": 42,
    "page_id": 1,
    "document_epoch": 1,
    "timestamp_ms": 1711100000000,
    "batch_id": "b_7",
    "source": "script",
    "mutations": [{
        "type": "childList|attributes|characterData",
        "target_node_id": "n45",
        "added?": ["n46"],
        "removed?": ["n12"],
        "parent_id?": "n45",
        "previous_sibling_id?": "n44",
        "attribute?": "class",
        "old_value?": "hidden",
        "new_value?": "visible"
    }]
}
```

---

## Implementation: What exists → NRP mapping

| NRP Command | Current Code | Status |
|------------|-------------|--------|
| Page.navigate | BrowserEngine::navigate() | ✅ exists |
| Interact.click | LiveDom::click() | ✅ exists (needs node_id) |
| Interact.type | LiveDom::type_text() | ✅ exists |
| Runtime.evaluate | JsRuntime::eval() | ✅ exists (needs object handles) |
| Cookies.import | ChromeCookieImporter | ✅ exists |
| Network.getLog | session.network_log | ✅ exists |
| Wait.forSelector | LiveDom::wait_for() | ✅ exists |
| SemanticTree.get | WOM extraction | ⚠️ needs restructure |
| Observe.subscribe | — | ❌ new |
| Runtime.callFunction | — | ❌ new |

### New crate: `neo-nrp`
- NRP types (ActionOutcome, SemanticNode, Target, etc.)
- NRP dispatcher (routes JSON-RPC → BrowserEngine)
- Event system (subscriptions, buffering, sequence IDs)
- Protocol version negotiation

### Timeline
- Week 1: Types + SemanticTree builder
- Week 2: Core commands dispatcher
- Week 3: Events + Network interception
- Week 4: Transport (WebSocket) + integration tests
