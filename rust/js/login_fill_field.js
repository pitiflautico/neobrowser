// Fill a login field through the framework-visible setter.
//
// Same reasoning as `fill_control`, kept separate because a login form is the case where
// getting it wrong is most expensive: the field looks filled, the submit sends empty
// credentials, and the result is indistinguishable from wrong credentials.

(function() {
    var el = document.querySelector('input[type=password]');
    if (!el) return;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
    if (setter && setter.set) setter.set.call(el, __V__); else el.value = __V__;
    el.dispatchEvent(new Event('input', {bubbles:true}));
    el.dispatchEvent(new Event('change', {bubbles:true}));
})()
