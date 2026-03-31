# PDR: Interaction Model — Event Dispatch + Default Actions + Framework Compat

## Core distinction
**Event dispatch ≠ browser action.** A real browser does TWO things on user input:
1. Dispatches events through the DOM (capture → target → bubble)
2. Executes **default actions** if not prevented (navigate on `<a>` click, toggle checkbox, submit form, etc.)

We were only doing #1. Frameworks receive events via delegation (#1) but the DOM doesn't mutate correctly without #2.

## Known limitations (honest)

### Hard limits (isTrusted=false)
Synthetic events have `isTrusted: false`. Cannot be changed. Blocked actions:
- File picker (`<input type="file">` click won't open dialog)
- Clipboard API (copy/paste via execCommand may fail)
- Fullscreen/share/notification requests
- Some anti-bot handlers checking isTrusted
- Some modal/dialog triggers

### Structural gaps (COMPAT, not semantic)
- **No hit-testing**: without real coordinates/layout, can't verify element is actually visible/clickable under overlays
- **No IME/composition**: only Latin append-mode typing. CJK excluded.
- **No caret/selection model**: backspace/delete only work as string truncation, not cursor-relative deletion. Insert-at-cursor not supported.
- **contenteditable**: partial (set innerHTML + dispatch input). No cursor, range, or rich editing.
- **No drag interactions**: requires real coordinates + pointermove sequences
- **Select element**: shortcut — direct value mutation + events, no open/close/dropdown semantics
- **beforeinput**: omitted from MVP. Instrumented as missing (trace log when would-be-needed).
- **change timing**: we fire change after type_text completes. Real browser fires on blur/commit. Documented as compat shortcut — may trigger premature side effects.

---

## F1: Default Action Model

After dispatching an event, if `event.defaultPrevented === false`, execute the browser's default action.

### Click default actions
| Element | Default action |
|---------|---------------|
| `<a href="...">` | See anchor rules below |
| `<a href="#id">` | Scroll to element (no-op in our case) |
| `<button type="submit">` | Submit parent form |
| `<button type="reset">` | Reset parent form |
| `<button>` (no type) | Submit parent form (default is submit) |
| `<input type="submit">` | Submit parent form |
| `<input type="checkbox">` | Toggle `.checked`, dispatch `input` + `change` |
| `<input type="radio">` | Set `.checked = true`, uncheck siblings in group, dispatch `input` + `change` |
| `<label>` | Forward click to associated control (for=id or nested) |
| `<select> <option>` | Set selected option, dispatch `input` + `change` |
| `<details> <summary>` | Toggle open attribute |
| `<input type="file">` | isTrusted limitation — cannot open file picker. Set files programmatically instead. |

### Anchor click rules
| Condition | Action |
|-----------|--------|
| `href` starts with `#` | Same-document hash navigation |
| `href` starts with `javascript:` | Eval the JS |
| `href` starts with `mailto:`/`tel:` | Log, don't navigate |
| `target="_blank"` or modifier keys (ctrl/meta) | Signal NewContext, don't navigate |
| `download` attribute | Signal download intent, don't navigate |
| `rel="noopener"` | Navigate normally (noopener only affects window.opener) |
| Otherwise | Full document navigation |

### Activation behavior
Default actions only fire on **activatable elements** after click. NOT on every element:
- Activatable: `<a>`, `<button>`, `<input>`, `<select>`, `<textarea>`, `<label>`, `<summary>`
- NOT activatable: `<div>`, `<span>`, `<p>`, etc.
- `<label>` forwarding: if click lands on label AND label's control is a different element → forward click to control. If click already on the control → do NOT double-fire.

### Submit with validation (requestSubmit path)
```
1. If submitter has formnovalidate → skip validation
2. Else: form.checkValidity() → if false, dispatch 'invalid' on failing inputs, abort submit
3. Create SubmitEvent (not generic Event — use SubmitEvent if available, else Event with .submitter polyfill)
4. Dispatch on form (does NOT bubble)
5. If not prevented → collect data + navigate
```

### Radio group scoping
Uncheck siblings only within the **same form owner** (or document if no form). Not all radios with same name globally.

Implementation: after `dispatchEvent(clickEvent)`, check `!clickEvent.defaultPrevented`, then execute action based on element type + activation rules.

### Submit default actions
**CRITICAL: submit event does NOT bubble in real browsers.** Our PDR had this wrong.
- `form.submit()` → no event, no validation, direct submit
- `form.requestSubmit(submitter?)` → fires submit event, runs constraint validation, THEN submits
- Submit button click → `requestSubmit(button)` semantics
- Enter in input → implicit submission (complex rules, see below)

Implementation:
```javascript
function executeSubmit(form, submitter) {
    // 1. Create submit event (does NOT bubble)
    const evt = new Event('submit', { bubbles: false, cancelable: true });
    evt.submitter = submitter || null;
    const prevented = !form.dispatchEvent(evt);
    if (prevented) return { action: 'prevented' };

    // 2. Collect form data
    const data = collectFormData(form, submitter);
    const action = submitter?.formAction || form.action || location.href;
    const method = (submitter?.formMethod || form.method || 'GET').toUpperCase();
    const enctype = submitter?.formEnctype || form.enctype || 'application/x-www-form-urlencoded';

    // 3. Navigate
    op_navigation_request({ url: action, method, form_data: data, enctype, type: 'form_submit' });
    return { action: action, method: method };
}
```

### Enter key implicit submission
NOT a simple "if form has submit button → submit". Real spec:
1. If the input's form owner has no submit button → implicit submission (submit the form)
2. If the form has a submit button → activate the first submit button (which triggers submit)
3. If `keydown` `Enter` is `preventDefault`'d → no submission
4. `<textarea>`: Enter inserts newline, NEVER submits
5. `<input type="search">`: Enter submits
6. `<input>` in general: Enter submits unless form has 0 or >1 text fields (complex, simplify for MVP)

MVP simplification: Enter on `<input>` submits if form has a submit button. Document as COMPAT, not full spec.

---

## F2: Event Sequences

### Click (full)
```
pointerover  (PointerEvent, bubbles) — if not already over
pointerenter (PointerEvent, does NOT bubble)
mouseover    (MouseEvent, bubbles) — if not already over
mouseenter   (MouseEvent, does NOT bubble)
pointermove  (PointerEvent, bubbles) — at least 1
mousemove    (MouseEvent, bubbles) — at least 1
pointerdown  (PointerEvent, bubbles)
mousedown    (MouseEvent, bubbles)
focus/focusin on target (if focusable, blur/focusout on previous)
pointerup    (PointerEvent, bubbles)
mouseup      (MouseEvent, bubbles)
click        (MouseEvent, bubbles)
→ IF NOT defaultPrevented: execute default action
```

MVP: skip pointerover/enter/mouseover/enter/move (they're for hover state). Keep from pointerdown onward. Document omission.

### Focus sequence (when element receives focus)
```
focusout (FocusEvent, bubbles, on OLD element)
blur     (FocusEvent, does NOT bubble, on OLD element)
focusin  (FocusEvent, bubbles, on NEW element)
focus    (FocusEvent, does NOT bubble, on NEW element)
→ Update document.activeElement
```

### Type (per character)
```
focus sequence (if not already focused)
For each character:
  keydown    (KeyboardEvent, key=char, code=Key+UPPER, bubbles)
  beforeinput (InputEvent, inputType='insertText', data=char, cancelable, bubbles) — if not cancelled:
  → mutate value (use native setter)
  input      (InputEvent, inputType='insertText', data=char, NOT cancelable, bubbles)
  keyup      (KeyboardEvent, key=char, code=Key+UPPER, bubbles)
After all chars:
  change     (Event, bubbles) — on blur or explicit
```

MVP: skip `beforeinput` (it's cancelable and few frameworks rely on it for basic inputs). But include it as a future addition. Document omission.

### Special keys
| Key | Events | Default action |
|-----|--------|---------------|
| Enter | keydown → (keypress legacy) → keyup | Submit form or newline in textarea |
| Tab | keydown → keyup | Move focus to next focusable element |
| Escape | keydown → keyup | Close dialog/modal if open |
| Backspace | keydown → beforeinput(deleteContentBackward) → input → keyup | Remove last char |
| Delete | keydown → beforeinput(deleteContentForward) → input → keyup | Remove next char |

`keypress`: legacy event. INCLUDE in sequence for Enter/printable chars. Some sites still depend on it. Position: after keydown, before input.

### Checkbox/radio toggle
**Checkbox click:**
```
full click sequence → if !defaultPrevented:
  toggle element.checked
  dispatch InputEvent('input', {bubbles:true})
  dispatch Event('change', {bubbles:true})
```

**Radio click:**
```
full click sequence → if !defaultPrevented:
  uncheck all radios in same name group
  set element.checked = true
  dispatch InputEvent('input', {bubbles:true}) on this radio
  dispatch Event('change', {bubbles:true}) on this radio
```

### Select change
```
set element.value = selectedValue
dispatch InputEvent('input', {bubbles:true})
dispatch Event('change', {bubbles:true})
```

---

## F3: Value setter (React compat)

React does NOT globally override `HTMLInputElement.prototype.value`. But React's synthetic event system tracks input values internally. To trigger React state update:

1. Call the **native prototype setter** to set the DOM value
2. Dispatch `InputEvent('input')` — React's delegation catches this

```javascript
function setNativeValue(el, value) {
    // Get the native setter from the prototype chain
    const proto = Object.getPrototypeOf(el);
    const desc = Object.getOwnPropertyDescriptor(proto, 'value')
        || Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')
        || Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
    if (desc && desc.set) {
        desc.set.call(el, value);
    } else {
        el.value = value; // fallback
    }
}
```

This is what Playwright/Puppeteer/testing-library do. Not a React-specific hack — it's the correct way to programmatically set input values.

---

## F4: Post-interaction pump

NOT fixed 10 rounds. Budget-based with tracing:

```rust
fn pump_after_interaction(&mut self) {
    if let Some(ref mut rt) = self.runtime {
        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(100); // 100ms max
        let mut rounds = 0;
        while start.elapsed() < budget {
            match rt.pump_event_loop() {
                Ok(true) => rounds += 1,  // did work
                Ok(false) => break,       // idle
                Err(_) => break,          // error
            }
        }
        if rounds > 0 {
            neo_trace!("[INTERACT] pumped {} rounds in {}ms", rounds, start.elapsed().as_millis());
        }
    }
}
```

---

## F5: Navigation coupling

After interaction, detect what happened:

| Signal | Meaning |
|--------|---------|
| URL unchanged, DOM unchanged | No effect |
| URL unchanged, DOM changed | Local state mutation (framework re-render) |
| URL changed via pushState | SPA route change (no HTTP needed) |
| Navigation request in queue | Full page navigation (HTTP needed) |
| Window.open detected | New context (not followed) |

Check AFTER pump, BEFORE returning to caller.

---

## Testing

### Fixture 1: Event delegation (vanilla)
```html
<button id="btn">Click me</button>
<div id="log"></div>
<script>
document.addEventListener('click', e => {
    document.getElementById('log').textContent = 'clicked:' + e.target.id;
});
</script>
```
LiveDom click → verify `#log` says "clicked:btn"

### Fixture 2: Controlled input (React-like delegation)
```html
<input id="inp" value="">
<div id="out"></div>
<script>
document.addEventListener('input', e => {
    if (e.target.id === 'inp') {
        document.getElementById('out').textContent = 'value:' + e.target.value;
    }
});
</script>
```
LiveDom type → verify `#out` shows typed text

### Fixture 3: Checkbox toggle
```html
<input type="checkbox" id="cb">
<div id="cblog"></div>
<script>
document.getElementById('cb').addEventListener('change', e => {
    document.getElementById('cblog').textContent = 'checked:' + e.target.checked;
});
</script>
```
LiveDom click on checkbox → verify `#cblog` says "checked:true"

### Fixture 4: Form submit (non-bubbling)
```html
<form id="f" action="/test"><input name="q" value="hello"><button type="submit">Go</button></form>
<div id="sublog"></div>
<script>
document.getElementById('f').addEventListener('submit', e => {
    e.preventDefault();
    document.getElementById('sublog').textContent = 'submitted:' + new FormData(e.target).get('q');
});
</script>
```
LiveDom click submit button → verify `#sublog` says "submitted:hello"

### Real site test
DuckDuckGo: type + submit → page navigates to results

## Gate (per-action metrics)

For each interaction test:
1. **Target found** — element resolved
2. **Events dispatched** — correct sequence (trace log)
3. **Default action fired** — checkbox toggled, form submitted, link navigated
4. **DOM delta** — measurable DOM change after interaction
5. **Framework state delta** — delegation listener received event (fixture tests)
6. **Navigation delta** — if applicable, URL changed or nav request queued

### Passing criteria
- All 4 fixtures pass
- DuckDuckGo type+submit navigates
- Checkbox toggle works
- Submit event does NOT bubble (verified)
- Post-interaction pump runs until idle (not fixed rounds)
- No regressions on 11/11 sites
