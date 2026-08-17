// Stop recording and restore the page's original console.
//
// Restoring matters: leaving the wrapper installed means every later observation of this page
// carries an instrumentation the caller did not ask for.

if (window.__neo_debug_orig) {
    console.log = window.__neo_debug_orig.log;
    console.warn = window.__neo_debug_orig.warn;
    console.error = window.__neo_debug_orig.error;
    delete window.__neo_debug_orig;
}
window.__neo_debug_logs = [];
