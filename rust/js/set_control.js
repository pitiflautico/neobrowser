// Set a checkbox, radio, <select> or contenteditable.
//
// Everything goes through the prototype's `value`/`checked` setter. A bare `el.checked =
// true` does not notify React or Vue, so the control visually changes and the app never
// learns — the classic "the form submitted the old value" bug.
//
// Placeholders: __SEL__ (JSON string), __VALUE__ (JSON string).
(function () {
  var e = document.querySelector(__SEL__);
  if (!e) return JSON.stringify({ ok: false, error: 'element not found' });

  var v = __VALUE__;
  var tag = e.tagName.toLowerCase();

  function notify(el) {
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  if (tag === 'select') {
    var matched = null;
    for (var i = 0; i < e.options.length; i++) {
      var o = e.options[i];
      // Matched by value OR visible text, because a caller naturally writes what they see.
      if (o.value === v || (o.text || '').trim() === v) {
        matched = o;
        break;
      }
    }
    if (!matched) {
      var available = [];
      for (var j = 0; j < e.options.length && j < 30; j++) available.push(e.options[j].value);
      return JSON.stringify({ ok: false, error: 'no option matched', available: available });
    }
    var d = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value');
    if (d && d.set) d.set.call(e, matched.value);
    else e.value = matched.value;
    notify(e);
    return JSON.stringify({ ok: true, control: 'select', value: e.value });
  }

  if (e.type === 'checkbox' || e.type === 'radio') {
    var want = v === 'true' || v === '1' || v === 'on' || v === 'checked';
    var cd = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'checked');
    if (cd && cd.set) cd.set.call(e, want);
    else e.checked = want;
    notify(e);
    return JSON.stringify({ ok: true, control: e.type, checked: e.checked });
  }

  if (e.isContentEditable) {
    e.focus();
    e.textContent = v;
    notify(e);
    return JSON.stringify({ ok: true, control: 'contenteditable', length: v.length });
  }

  return JSON.stringify({ ok: false, error: 'not a checkbox, radio, select or contenteditable: ' + tag });
})()
