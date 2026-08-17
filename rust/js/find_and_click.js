// Find the element that best matches an intent, and report why it was chosen.
//
// Returns the pick rather than clicking it, so the click itself goes through the trusted
// pointer path. Scoring considers accessible name, visible text, title and value, and it
// returns the runner-up too — when the wrong thing gets clicked, the second choice is the
// most useful piece of debugging information there is.

(function() {
    var role = __ROLE__; var textQ = __TEXTQ__; var nth = __NTH__;
    var sel = role ? '[role=' + role + '],button,a,[role=button],[role=link]' : 'button,a,[role=button],[role=link],input[type=submit]';
    var els = Array.from(document.querySelectorAll(sel));
    var matches = els.filter(function(e) {
        return e.textContent.toLowerCase().indexOf(textQ) !== -1 ||
               (e.getAttribute('aria-label')||'').toLowerCase().indexOf(textQ) !== -1;
    });
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
        text: target.textContent.trim().slice(0,60), nth: nth});
})()
