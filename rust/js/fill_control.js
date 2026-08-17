// Set a field's value the way a framework will notice.
//
// Assigning `.value` directly is not enough: React and Vue track state through the property
// descriptor on the prototype, so a direct assignment updates the DOM and leaves the
// component's state stale — the field looks filled and submits empty. So this calls the
// prototype's own setter and then dispatches the `input`/`change` events the framework
// listens for. Selects, checkboxes, radios and contenteditable each need a different one of
// those three steps, which is why they are branches rather than one path.

(function() {
    var sel = __SEL__; var v = __VAL__;
    var el = document.querySelector(sel);
    if (!el) return JSON.stringify({ok: false, error: "selector not found"});
    var tag = el.tagName.toLowerCase();
    var type = (el.type || '').toLowerCase();
    if (tag === 'select') {
        el.value = v; el.dispatchEvent(new Event('change', {bubbles: true}));
    } else if (type === 'checkbox' || type === 'radio') {
        el.checked = (v === 'true' || v === true);
        el.dispatchEvent(new Event('change', {bubbles: true}));
    } else if (el.isContentEditable) {
        el.focus(); el.textContent = v;
        el.dispatchEvent(new Event('input', {bubbles: true}));
        return JSON.stringify({ok: true, tag: tag, type: 'contenteditable', value: el.textContent});
    } else {
        var proto = tag === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
        var setter = Object.getOwnPropertyDescriptor(proto, 'value');
        if (setter && setter.set) { setter.set.call(el, v); } else { el.value = v; }
        el.dispatchEvent(new Event('input', {bubbles: true}));
        el.dispatchEvent(new Event('change', {bubbles: true}));
    }
    return JSON.stringify({ok: true, tag: tag, type: type, value: el.value});
})()
