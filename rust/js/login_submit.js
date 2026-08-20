// Submit the login form.

(function() {
    var pw = document.querySelector('input[type=password]');
    var form = pw && pw.form;
    var btn = form
        ? form.querySelector('button[type=submit],input[type=submit]')
        : document.querySelector('button[type=submit],input[type=submit]');
    if (btn) btn.click();
    else if (form) form.submit();
    else { var f = document.querySelector('form'); if (f) f.submit(); }
})()
