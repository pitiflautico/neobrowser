# PDR: Runtime Completion for SPA Mount (General Engine Level)

## Problem (framed correctly)
Even after Cloudflare bypass + full HTML fetch + dynamic imports succeeding, many SPAs remain at SSR shell only. The engine reaches "0 JS errors" but the application never reaches interactive state. This blocks real AI interaction.

No framework assumptions allowed. All gates and diagnostics must be framework-agnostic.

## Current Capabilities vs Reality (no inflation)

| Claim (old) | Reality (new) |
|------------|---------------|
| Navigate to any URL 11/11 | Tested on 11 representative sites only |
| Set input values ✅ | Basic DOM .value works; framework-controlled inputs do not update internal state |
| Click with default actions ✅ | Native button/anchor works; delegated framework handlers do not fire |

## Root Cause (still open — investigation mandatory)

Dynamic imports + module evaluation complete, but one or more of:
1. Event loop never reaches idle-or-budget after last module
2. Critical post-load fetch / promise rejection silent
3. Missing browser API called during hydration (not during parse)
4. Delegated event handlers never attached

No hypothesis sold as fact. We will prove it with traces.

## Required Additions

### 1. Form Constraint Validation (must be real)
```javascript
input.willValidate
input.checkValidity()
input.reportValidity()
input.setCustomValidity()
// invalid / valid events
form.checkValidity()
form.reportValidity()
```

### 2. Selection / Caret State
```javascript
input.selectionStart / selectionEnd / selectionDirection
document.getSelection()  // minimal stub returning real Range on contenteditable
input.setSelectionRange()
```

### 3. Contenteditable Model (limited but required)
- contenteditable="true" elements must support innerHTML mutation + selection
- document.execCommand polyfill for 'insertText' / 'insertHTML' if needed

### 4. Action Result Attribution (default vs delegated)
```rust
struct ActionResult {
    success: bool,
    changed_dom: bool,             // visible DOM delta
    framework_handler_fired: bool, // detected via MutationObserver + timing
    default_action_taken: bool,
    error: Option<String>,
}
```

## Phases

### Phase 1: Runtime Completion for SPA Mount

Gate (ALL must pass — framework agnostic):
- All dynamic import() chains complete (no pending fetch in trace)
- Module evaluation settles (mod_evaluate future resolved + no unhandled rejection)
- Event loop reaches idle-or-budget (run_event_loop until !did_work for ≥3 consecutive turns)
- DOM post-hydration delta > SSR baseline (node count increase + new elements with data-* or aria-* attributes)
- At least one delegated interaction produces observable app-state change (WOM diff before/after click/type)

Diagnostics to inject before any script:
```javascript
window.__neorender_trace = {
    modulesLoaded: 0,
    pendingFetches: new Set(),
    lastMutationTime: Date.now()
};
new MutationObserver(() => {
    window.__neorender_trace.lastMutationTime = Date.now();
}).observe(document.body, {childList:true, subtree:true});
```

### Phase 2: WOM Enriched
- formValidity object per form
- selection state per editable element
- postHydrationDelta (node count + mutation timestamp)

### Phase 3: Form Filler + Validation Aware
Use constraint APIs before submitting:
- checkValidity() on form
- reportValidity() feedback
- Skip invalid fields with clear error to AI

### Phase 4: Action Result Tracking
Returns ActionResult with:
- framework_handler_fired (detected via timing + MutationObserver delta within 50ms)
- default_action_taken
- DOM delta

### Phase 5: Verification (no framework probing)
Gate: "dynamic import chain completes + event loop idle + DOM delta > 200 nodes + delegated click changes WOM state"
Tested on: ChatGPT, Mercadona, Vercel, Nuxt demo, SvelteKit demo (no framework names in gates)

## Implementation Order

1. Add 4 missing pieces (validation + selection + contenteditable + attribution)
2. Inject global trace + MutationObserver in runtime init
3. Extend settle phase to idle-or-budget (conditional on lastMutationTime)
4. Update ActionResult struct + WOM serializer
5. Run Phase 1 gate on 11 sites + 3 new SPA benchmarks

## What NOT to do

- Never put framework internals (React fiber, hydrateRoot, Vue app) in any gate or diagnostic
- Never claim "any URL" or "full interaction" until gate passes on >50 sites
- Never ship framework-specific patches
