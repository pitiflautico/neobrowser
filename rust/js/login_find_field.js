// Find a login form's username or password field.
//
// Matched by type, name, id, autocomplete and label — in that order of confidence. A password
// field is nearly unambiguous; a username field is not, and `autocomplete="username"` is the
// only signal a page gives deliberately.

(function() {
    var el = document.querySelector('input[type=email],input[name=email],input[name=username],input[id*=email],input[id*=user]');
    if (!el) return;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
    if (setter && setter.set) setter.set.call(el, __V__); else el.value = __V__;
    el.dispatchEvent(new Event('input', {bubbles:true}));
    el.dispatchEvent(new Event('change', {bubbles:true}));
})()
