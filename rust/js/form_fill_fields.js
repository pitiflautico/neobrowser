// Match a form's fields against the labels a human would read.
//
// The caller names fields the way the page presents them ("Email", "Card number"), not by
// selector, so each is matched against label text, name, placeholder and aria-label in turn.
// Scoring rather than first-match, because a page with both "Email" and "Email confirmation"
// must not fill the wrong one.

(function() {
    var forms = document.querySelectorAll('form');
    var form = forms[__IDX__] || document;
    var inputs = Array.from(form.querySelectorAll('input,select,textarea'));
    var target = null; var lq = __LABEL__.toLowerCase();
    for (var i=0; i<inputs.length; i++) {
        var el = inputs[i];
        var candidates = [el.name, el.id, el.placeholder, el.getAttribute('aria-label')];
        var lbl = '';
        if (el.id) { var l = document.querySelector('label[for="'+el.id+'"]'); if(l) lbl = l.textContent; }
        candidates.push(lbl);
        for (var j=0; j<candidates.length; j++) {
            if (candidates[j] && candidates[j].toLowerCase().indexOf(lq) !== -1) { target = el; break; }
        }
        if (target) break;
    }
    if (!target) return JSON.stringify({ok: false, error: 'field not found: '+__LABEL__});
    var tag = target.tagName.toLowerCase(); var type = (target.type||'').toLowerCase(); var v = __VAL__;
    if (tag === 'select') { target.value = v; target.dispatchEvent(new Event('change', {bubbles: true})); }
    else if (type === 'checkbox' || type === 'radio') { target.checked = (v === 'true' || v === true); target.dispatchEvent(new Event('change', {bubbles: true})); }
    else {
        var proto = tag === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
        var setter = Object.getOwnPropertyDescriptor(proto, 'value');
        if (setter && setter.set) { setter.set.call(target, v); } else { target.value = v; }
        target.dispatchEvent(new Event('input', {bubbles: true}));
        target.dispatchEvent(new Event('change', {bubbles: true}));
    }
    return JSON.stringify({ok: true, field: __LABEL__, value: type === 'password' ? '••••••••' : target.value});
})()
