// Find the element that best matches an intent, and report why it was chosen.
//
// Returns the pick rather than clicking it, so the click itself goes through the trusted
// pointer path. It returns the runner-up too — when the wrong thing gets clicked, the second
// choice is the most useful piece of debugging information there is.
//
// Matching considers every place a control's visible label can live, and that list is longer
// than it looks. `<input type="submit" value="Login">` has no textContent at all: the word the
// user sees is in `value`. An earlier version matched only textContent and aria-label, so it
// could not find the submit button on a large fraction of the real web — including the login
// form of the app this project uses as a real-site test. The accessibility tree gets this
// right (`observe` reported the control correctly the whole time), which is why the two
// disagreed, and why the fix is to match what a user reads rather than what the DOM stores.

(function() {
    var role = __ROLE__; var textQ = __TEXTQ__; var nth = __NTH__;
    // Every control a user would call a button, including the four <input> types that are
    // one. Listing only input[type=submit] missed `<input type=button value=Cancel>`, which
    // is as common as the submit variant.
    var CLICKABLE = 'button,a,[role=button],[role=link],' +
                    'input[type=submit],input[type=button],input[type=reset],input[type=image],' +
                    'summary,[onclick]';
    var sel = role ? '[role=' + role + '],' + CLICKABLE : CLICKABLE;
    var els = Array.from(document.querySelectorAll(sel));
    // Every place a visible label can live, in the order a user would read them.
    var labelOf = function(e) {
        return [
            e.textContent,
            e.getAttribute('aria-label'),
            e.value,                        // <input type=submit|button|reset>
            e.getAttribute('title'),
            e.getAttribute('alt'),          // <input type=image>
            e.getAttribute('placeholder')
        ].filter(Boolean).join(' ').toLowerCase();
    };
    var matches = els.filter(function(e) { return labelOf(e).indexOf(textQ) !== -1; });
    var total = matches.length;
    var visible = matches.filter(function(e) {
        var r = e.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return false;
        var s = getComputedStyle(e);
        if (s.visibility === 'hidden' || s.display === 'none' || s.opacity === '0') return false;
        // Reject anything inside a collapsed ancestor — the accordion case.
        // Stop before <body>/<html>: those routinely measure zero height with
        // overflow:hidden on sites using fixed or virtualised scrolling, and
        // treating that as "collapsed" would hide every element on the page.
        for (var p = e.parentElement;
             p && p !== document.body && p !== document.documentElement;
             p = p.parentElement) {
            var pr = p.getBoundingClientRect();
            if (pr.height === 0 || pr.width === 0) {
                if (getComputedStyle(p).overflow !== 'visible') return false;
            }
        }
        return true;
    });
    if (total === 0)
        return JSON.stringify({ok: false, error: "no match for: " + __TEXTRAW__});
    if (visible.length === 0)
        return JSON.stringify({ok: false, matched_total: total, matched_visible: 0,
            error: "matched " + total + " node(s) for " + __TEXTRAW__ +
                   ", all hidden or inside a collapsed container"});
    var target = visible[Math.min(nth, visible.length-1)];
    window.__nbClickTarget = target;
    return JSON.stringify({ok: true, matched_total: total,
        matched_visible: visible.length,
        text: (target.textContent.trim() || target.value || target.getAttribute('aria-label') || '').slice(0,60), nth: nth});
})()
