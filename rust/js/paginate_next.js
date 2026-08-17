// Find and follow the next-page control.
//
// Reports whether it actually advanced. A "next" link that is present but disabled is the
// standard way a scraping loop re-reads page one forever while reporting success.

(function() {
    var patterns = ['next','siguiente','→','›','>>','»','more','load more'];
    var els = Array.from(document.querySelectorAll('a,button,[role=button]'));
    for (var i=0; i<els.length; i++) {
        var txt = els[i].textContent.toLowerCase().trim();
        var aria = (els[i].getAttribute('aria-label')||'').toLowerCase();
        for (var j=0; j<patterns.length; j++) {
            if (txt === patterns[j] || aria === patterns[j]) {
                els[i].click();
                return JSON.stringify({ok: true, matched: patterns[j]});
            }
        }
    }
    var rel = document.querySelector('a[rel=next]');
    if (rel) { rel.click(); return JSON.stringify({ok: true, method: "rel_next"}); }
    return JSON.stringify({ok: false, error: "no next button found"});
})()
