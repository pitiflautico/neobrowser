// Follow a caller-supplied next-page control.
//
// The explicit-selector half of `paginate`: when the caller names the control there is
// nothing to guess, so the only question worth answering is whether the selector matched
// anything at all. Saying so is the point — a click on nothing looks exactly like a click on
// a disabled "next", and both leave a scraping loop re-reading page one while reporting
// success.

(function() {
    var el = document.querySelector(__SEL__);
    if (!el) return JSON.stringify({ok: false, error: "selector not found"});
    el.click();
    return JSON.stringify({ok: true, method: "custom_selector"});
})()
