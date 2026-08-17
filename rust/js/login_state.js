// Decide whether a login finished, and say when it cannot tell.
//
// The three answers are not "yes" and "no" but "signed in", "still on the form", and
// "cannot tell" — because a page mid-2FA, mid-validation and outright rejected all look the
// same from outside. Collapsing "cannot tell" into either of the others is what makes an agent
// proceed as though authenticated when it is not.

(function() {
    return Array.from(document.querySelectorAll('input[type=password]'))
        .some(function(el) {
            var r = el.getBoundingClientRect();
            if (r.width === 0 || r.height === 0) return false;
            var s = getComputedStyle(el);
            return s.visibility !== 'hidden' && s.display !== 'none';
        });
})()
