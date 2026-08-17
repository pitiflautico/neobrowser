// Start recording console output and errors in the page.
//
// Installed as a wrapper rather than a replacement, so the page's own logging keeps working —
// a page whose `console.log` stopped behaving would change the behaviour being debugged.

if (!window.__neo_debug_logs) window.__neo_debug_logs = [];
window.__neo_debug_orig = {log: console.log, warn: console.warn, error: console.error};
['log','warn','error'].forEach(function(l) {
    console[l] = function() {
        var msg = Array.from(arguments).map(function(a){ try{return JSON.stringify(a);}catch(e){return String(a);} }).join(' ');
        window.__neo_debug_logs.push({level: l, msg: msg, t: Date.now()});
        window.__neo_debug_orig[l].apply(console, arguments);
    };
});
