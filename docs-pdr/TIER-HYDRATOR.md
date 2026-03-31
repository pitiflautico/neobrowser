# TIER: Virtual Hydrator — 4 Fases, 21 Sub-fases (Grok-reviewed)

## Execution Order (corrected)

```
FASE 1: Foundation
  F1a  WOM enrichment (15 new fields)
  F1c  Focus model (dispatch events on focus/blur, focusin bubbles, focus doesn't)
  F1b  GET/POST submit (full serialization)

FASE 2: Interaction
  F2d  Selection/caret minimal
  F2f  Selection replacement (write over selection)
  F2b  Backspace/delete
  F2c  beforeinput event (insertText, deleteContent*, insertLineBreak)
  F2e  Change on blur (not on type_text end)
  F2a  Tab focus cycling (correct tab order: tabindex>0 asc → tabindex=0 DOM order)

FASE 3: Form engine
  F3b  Select/option model (select-one, select-multiple, selectedIndex sync)
  F3c  Radio group scoping (same form owner)
  F3f  Constraint validation (required, pattern, email, min/max, invalid event, block submit)
  F3a  Enctype support (urlencoded, multipart, text/plain)
  F3d  Details toggle
  F3e  Escape dismiss

FASE 4: Pump + Observability
  F4a  Interaction pump with fetch drain (instrumented counters)
  F4b  Per-action trace (events, default action, focus, value, DOM delta)
  F4c  Structured navigation result (http_nav, spa_nav, dom_only, no_effect)
  F4d  DOM IDL shims (labels, control, selectedOptions)
  F4e  Action classification (21 distinct outcomes)
```

---

## FASE 1: Foundation

### F1a: WOM enrichment
Add to WomNode struct: `input_type`, `name`, `checked`, `selected`, `required`, `disabled`, `readonly`, `placeholder`, `pattern`, `min`, `max`, `minlength`, `maxlength`, `autocomplete`, `form_id`, `valid`, `validation_message`, `options` (for select).

Files: `wom.rs`, `wom_builder.rs`
Test: fixture with all input types → WOM JSON has all attributes.
Gate: httpbin forms page WOM shows type, name, required on all fields.

### F1c: Focus model
Fix focus()/blur() shims to dispatch events:
- `focus()` → dispatch focusin (bubbles:true) + focus (bubbles:false)
- `blur()` → dispatch focusout (bubbles:true) + blur (bubbles:false)
- Update `document.activeElement` BETWEEN focusout and focusin (spec order)

Gate (2 assertions):
- `focusin` reaches document listener ✅
- `focus` does NOT reach document listener ✅

Files: `browser_shim.js`

### F1b: GET/POST submit (full serialization)
Fix form data collection to be spec-compliant:
- Skip disabled controls
- Checkboxes/radios: only include if checked
- Multiple values with same name → array
- Submitter name=value included
- `<textarea>` value included
- `<select multiple>` → all selected option values
- `form` attribute (controls outside `<form>` but associated via form=id)
- `<input type=image>` → documented exclusion

POST encoding:
- `application/x-www-form-urlencoded` → key=value&key=value in request body
- Request body sent via `op_navigation_request` with `body` field
- pipeline.rs handles POST body in HTTP request (fix the TODO)

GET: append as query string (already works).

Files: `live_dom.rs` (executeFormSubmit), `pipeline.rs` (process_pending_navigations)
Test: httpbin.org/post with disabled fields, unchecked checkboxes, textarea, select.
Gate: httpbin POST echo matches expected field set exactly.

---

## FASE 2: Interaction

### F2d: Selection/caret minimal
Scope: text inputs and textareas ONLY. NOT contenteditable.
- Track selectionStart/selectionEnd after each keystroke
- After insert: caret = end of inserted text
- `setSelectionRange(start, end)` works
- No selectionDirection for MVP

Files: `browser_shim.js` (selection APIs already shimmed), `live_dom.rs` (update caret in fireTypeText)
Gate: `setSelectionRange(2,2)` → type "X" → value has X at position 2.

### F2f: Selection replacement
When selectionStart ≠ selectionEnd and user types:
- Replace range [start, end] with typed text
- New caret position = start + typed text length
- Dispatch beforeinput with inputType='insertText'

Gate: value "hello", setSelectionRange(1,4), type "X" → "hXo", caret at 2.

### F2b: Backspace/delete
- Backspace: if selection collapsed → delete char before caret, else delete selection
- Delete: if selection collapsed → delete char after caret, else delete selection
- inputType: 'deleteContentBackward' / 'deleteContentForward'
- Update caret position

Files: `live_dom.rs` (firePressKey)
Gate: "hello" → backspace → "hell". "hello" sel[1,4] → backspace → "ho".

### F2c: beforeinput event
Dispatch BEFORE DOM mutation in type/backspace/delete/enter:
- `new InputEvent('beforeinput', {inputType, data, cancelable:true, bubbles:true})`
- inputTypes: `insertText`, `deleteContentBackward`, `deleteContentForward`, `insertLineBreak` (Enter in textarea)
- If `event.defaultPrevented` → skip mutation + skip input event

Files: `live_dom.rs`
Gate: listener cancels beforeinput → value unchanged.

### F2e: Change on blur
- Remove change dispatch from end of fireTypeText
- Track "dirty" flag per element: set true when input event fires
- When element loses focus (focusout in focus sequence): if dirty → dispatch change → clear dirty

Files: `live_dom.rs` (fireTypeText, fireClick focus sequence)
Gate: type → no change yet → click elsewhere → change fires.

### F2a: Tab focus cycling
Tab order (correct spec):
1. Collect all focusable elements: `input:not([disabled]):not([type=hidden]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])`
2. Sort: tabindex > 0 first (ascending), then tabindex = 0 in DOM order
3. tabindex = -1: focusable by script but NOT by Tab
4. Shift+Tab: reverse order

On Tab:
1. Find current position in tab order
2. Move to next (or previous for Shift+Tab)
3. Execute focus sequence (blur old → focus new)

Files: `live_dom.rs` (firePressKey Tab handler)
Gate: 3 inputs with tabindex 0,2,1 → Tab order is: tabindex=1, tabindex=2, tabindex=0.

---

## FASE 3: Form Engine

### F3b: Select/option model
- `select-one`: value = first selected option, change = set value + update selectedIndex
- `select-multiple`: selectedOptions = all selected, toggle with ctrl+click
- `selectedIndex` property synced with `option.selected`
- `select.value` synced with `option[selected]`
- Default: first option selected if none marked
- WOM: options list with value, text, selected for each

Files: `live_dom.rs`, `browser_shim.js`, `wom_builder.rs`
Gate: WOM shows select options. Set value → selectedIndex updates.

### F3c: Radio group scoping
Uncheck siblings only within same form owner:
- If radio has `form` attribute → scope to that form's radios
- If radio is inside `<form>` → scope to that form
- If radio is outside any form → scope to document

Files: `live_dom.rs` (fireClick radio section)
Gate: two forms, same radio name → click in form1 doesn't affect form2.

### F3f: Constraint validation
- `checkValidity()` on form: checks all fields
- If invalid: dispatch `invalid` event on each failing control
- `requestSubmit()`: validate first → if invalid, stop → if valid, submit
- Submit via button click: uses requestSubmit path (validates)
- `form.submit()`: NO validation (spec behavior)
- Supported constraints: required, type=email, type=url, pattern, minlength, maxlength, min/max (number)
- `validationMessage` filled with human-readable message

Files: `browser_shim.js` (already has basic checkValidity — extend), `live_dom.rs` (submit path)
Gate: required field empty → submit blocked → invalid event fires → validationMessage set.

### F3a: Enctype complete
- `application/x-www-form-urlencoded` (default): key=value&key=value
- `multipart/form-data`: boundary + MIME parts
- `text/plain`: key=value\r\n (rare, low priority)

Files: `pipeline.rs`
Gate: multipart POST to httpbin returns file-like data structure.

### F3d: Details toggle
Click on `<summary>` → toggle parent `<details>` open attribute.
If open: remove attribute. If closed: set attribute.

Files: `live_dom.rs` (fireClick)
Gate: click summary → details.open toggles.

### F3e: Escape dismiss
Escape key:
1. Check for `<dialog open>` or `[role=dialog][aria-modal=true]` → close it
2. Remove open attribute / set display:none
3. If nothing to dismiss → no-op

Files: `live_dom.rs` (firePressKey)
Gate: open dialog + Escape → dialog closes.

---

## FASE 4: Pump + Observability

### F4a: Interaction pump with fetch drain
After each action:
1. Pump microtasks (V8 event loop)
2. Check `TaskTracker.fetches` — if >0, wait up to 2s
3. Check `__neorender_trace.lastMutationTime` — if recent, keep pumping
4. Budget: 2s total max
5. Log: rounds, fetches waited, mutations observed

Files: `pipeline.rs`, `browser_impl.rs`
Gate: click triggers fetch → pump waits → DOM updates from fetch response.

### F4b: Per-action trace
Every action result includes:
```json
{
  "events_dispatched": ["pointerdown","mousedown","pointerup","mouseup","click"],
  "default_action": "form_submit" | "navigation" | "checkbox_toggle" | "cancelled" | "none",
  "focus_before": "#email",
  "focus_after": "#password",
  "value_before": "",
  "value_after": "hello",
  "dom_delta": 3,
  "fetches_triggered": 0,
  "navigation_triggered": false
}
```

Files: `live_dom.rs` (dispatcher), `browser_impl.rs`
Gate: NEORENDER_TRACE=1 → action trace for click + type visible.

### F4c: Structured navigation result
Replace ActionOutcome with richer classification:
```rust
enum NavigationResult {
    HttpNavigation { url: String, method: String },
    SpaRouteChange { url: String },
    DomOnlyUpdate { mutations: usize },
    NoEffect,
}
```

Files: `live_dom.rs`, `pipeline.rs`
Gate: form submit returns HttpNavigation. pushState returns SpaRouteChange.

### F4d: DOM IDL shims
- `input.labels` → querySelectorAll(`label[for=id]`) + ancestor label
- `label.control` → getElementById(label.htmlFor) or first input/select/textarea descendant
- `select.selectedOptions` → filter options by selected property

Files: `browser_shim.js`
Gate: `input.labels.length` returns 1 for labeled input.

### F4e: Action classification
Distinct per-action result types for AI consumption:
```
dom_only, default_action_cancelled, validation_blocked,
http_submit, http_navigate, spa_route_change,
dialog_closed, toggle_changed, value_changed,
checkbox_toggled, radio_selected, option_selected,
focus_moved, no_effect
```

Files: `live_dom.rs`
Gate: each action type returns correct classification.

---

## Testing Strategy

### Per sub-fase: fixture tests (primary gate)
HTML fixtures in `tests/fixtures/` with specific controls and listeners.
Each fixture tests ONE capability. No external dependencies.

### After each fase: site smoke test (secondary)
1. httpbin.org/forms/post
2. DuckDuckGo
3. HN login
4. react.dev
5. ChatGPT

Smoke tests are NOT gates. They're indicators. Failures → diagnose which subfase is incomplete.

---

## What NOT to do
- Don't test on ChatGPT as primary gate (unstable, complex)
- Don't add framework-specific logic
- Don't skip fixture tests
- Don't mark anything ✅ without running the gate assertion
