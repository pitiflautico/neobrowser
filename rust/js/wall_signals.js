// Detect the things that stop automation: consent walls, captchas, logins, paywalls.
//
// Reported rather than worked around. Every one of these is a deliberate signal from the site,
// and the honest outcome is `NeedsHuman` with the reason named — a tool that silently defeats
// a captcha is a tool nobody can safely run against a site they do not own.

JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: (document.body ? document.body.innerText.slice(0, 4000) : ''),
    captcha: (function() {
        // Only a VISIBLE challenge is a wall. Invisible captchas —
        // Stripe's anti-fraud hCaptcha, reCAPTCHA v3, Turnstile in
        // managed mode — sit on ordinary payment and login pages and
        // never ask the user anything. Reporting those tells the agent
        // to hand off to a human on a page with nothing in its way.
        var els = document.querySelectorAll(
            'iframe[src*="recaptcha"], iframe[src*="hcaptcha"],' +
            'iframe[title*="challenge" i], div.cf-turnstile,' +
            '#challenge-form, iframe[src*="turnstile"]');
        for (var i = 0; i < els.length; i++) {
            var e = els[i];
            var r = e.getBoundingClientRect();
            var s = getComputedStyle(e);
            // A real widget is at least checkbox-sized (~300x65).
            if (r.width > 40 && r.height > 40
                && s.visibility !== 'hidden' && s.display !== 'none'
                && s.opacity !== '0') return true;
        }
        return false;
    })(),
    password: !!document.querySelector('input[type=password]'),
    cf: /just a moment|checking your browser|cf-browser-verification/i.test(document.body ? document.body.innerText : '')
})
