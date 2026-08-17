// Every link on the page, capped.
//
// The cap is not politeness: a page with fifty thousand anchors would otherwise return a
// payload no model can read and no transport should carry.

JSON.stringify(Array.from(document.querySelectorAll('a[href]')).slice(0,100).map(function(a){
    return {text: a.textContent.trim().slice(0,80), href: a.href};
}));
