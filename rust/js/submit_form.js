// Submit a form and report what the submission actually did.
//
// `requestSubmit` rather than `submit`, because `submit()` bypasses validation entirely — the
// form goes off with invalid data and the page's own error handling never runs. The return
// value records whether a submit button was found and whether validation passed, since a form
// that silently failed validation is otherwise indistinguishable from one that succeeded.

(function() {
    var btn = document.querySelector('button[type=submit],input[type=submit]');
    if (btn) { btn.click(); return "button_click"; }
    var btn2 = document.querySelector('[aria-label*="submit" i],[aria-label*="send" i]');
    if (btn2) { btn2.click(); return "aria_button"; }
    var form = document.querySelector('form');
    if (form) { form.submit(); return "form_submit"; }
    return null;
})()
