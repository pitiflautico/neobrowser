// ═══════════════════════════════════════════════════════════════
// CONSENT AUTO-ACCEPT — dismisses cookie/consent dialogs automatically.
// Loaded AFTER browser.js. Called from Rust after every goto().
// ═══════════════════════════════════════════════════════════════

globalThis.__neo_auto_consent = function() {
    // Multi-language accept button patterns (text content, lowercase)
    // Ordered: "accept all" variants first (preferred), then single "accept"
    var acceptTexts = [
        // EN
        'accept all', 'accept all cookies', 'allow all', 'allow all cookies',
        'accept', 'allow', 'agree', 'i agree', 'i understand', 'got it', 'ok',
        // ES
        'aceptar todo', 'aceptar todas', 'aceptar todas las cookies',
        'permitir todo', 'permitir todas las cookies',
        'aceptar', 'permitir', 'acepto', 'de acuerdo', 'entendido',
        // FR
        'tout accepter', 'accepter tout', 'tout autoriser',
        'accepter', "j'accepte", 'autoriser',
        // DE
        'alle akzeptieren', 'alle zulassen', 'alle cookies akzeptieren',
        'akzeptieren', 'zustimmen', 'einverstanden',
        // IT
        'accetta tutto', 'accetta tutti', 'accetta tutti i cookie',
        'accetta', 'accetto',
        // PT
        'aceitar tudo', 'aceitar todos', 'aceitar todos os cookies',
        'aceitar', 'aceito', 'concordo',
        // NL
        'alles accepteren', 'alle cookies accepteren',
        'accepteren', 'akkoord',
        // Generic
        'consent', 'consentir',
    ];

    // Well-known selectors for consent frameworks
    var selectors = [
        // OneTrust
        '#onetrust-accept-btn-handler',
        '#accept-recommended-btn-handler',
        '.onetrust-close-btn-handler',
        // CookieBot
        '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll',
        '#CybotCookiebotDialogBodyButtonAccept',
        'a#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll',
        // Quantcast
        '.qc-cmp2-summary-buttons button:first-child',
        '[data-tracking-opt-in-accept]',
        // StackOverflow / Amazon
        '#sp-cc-accept',
        // CookieConsent.js
        '.cc-btn.cc-allow',
        '.cc-accept-all',
        // Google consent (consent.google.com)
        'button[aria-label="Accept all"]',
        'button[aria-label="Aceptar todo"]',
        'button[aria-label="Tout accepter"]',
        'button[aria-label="Alle akzeptieren"]',
        // Didomi
        '#didomi-notice-agree-button',
        '.didomi-continue-without-agreeing',
        '[data-testid="notice-accept-btn"]',
        // TrustArc
        '#truste-consent-button',
        '.truste_popframe .truste-consent-button',
        '#consent-banner .accept',
        '.trustarc-agree-btn',
        // Usercentrics
        '#uc-btn-accept-banner',
        '[data-testid="uc-accept-all-button"]',
        '.uc-accepting-btn',
        // Borlabs Cookie (WordPress)
        '.BorlabsCookie ._brlbs-btn-accept-all',
        '#BorlabsCookieBox .cookie-accept',
        'a[data-cookie-accept]',
        // GDPR Cookie Compliance (WordPress)
        '.moove-gdpr-infobar-allow-all',
        '#moove_gdpr_cookie_modal .mgbutton',
        // Complianz (WordPress)
        '.cmplz-accept',
        '.cmplz-btn.cmplz-accept',
        // Cookie Notice (WordPress)
        '#cn-accept-cookie',
        '.cn-set-cookie',
        // Klaro
        '.klaro .cm-btn-accept',
        '.klaro .cm-btn-accept-all',
        // CookieYes / CookieLaw
        '#cookie_action_close_header',
        '.cky-btn-accept',
        '#cky-btn-accept',
        // Termly
        '[data-tid="banner-accept"]',
        '.t-consentPrompt-acceptAll',
        // Iubenda
        '.iubenda-cs-accept-btn',
        '#iubenda-cs-banner .iubenda-cs-accept-btn',
        // Osano
        '.osano-cm-accept-all',
        '.osano-cm-dialog__button--type_accept',
        // Cookiebot-like
        '.CookieDeclarationType .CookieDeclarationDialogButtonAcceptAll',
        // Common data attributes
        '[data-testid="cookie-accept"]',
        '[data-testid="accept-cookies"]',
        '[data-cookiebanner="accept_button"]',
        '[data-action="accept"]',
        '[data-gdpr="accept"]',
        '[data-consent="accept"]',
        // Common classes
        '.cookie-consent-accept',
        '#cookie-consent-accept',
        '.js-cookie-consent-agree',
        '.cookie-notice-accept',
        '.cookie-accept-all',
        '.cookie-accept-btn',
        '.consent-accept',
        '.consent-accept-all',
        // GDPR frameworks
        '.gdpr-accept',
        '#gdpr-consent-accept',
        '.gdpr-consent-accept-all',
    ];

    // === Phase 1: Try well-known selectors (most reliable) ===
    for (var si = 0; si < selectors.length; si++) {
        try {
            var el = document.querySelector(selectors[si]);
            if (el) {
                el.click && el.click();
                el.dispatchEvent(new MouseEvent('click', {bubbles: true}));
                return JSON.stringify({ok: true, method: 'selector', match: selectors[si]});
            }
        } catch (e) {}
    }

    // === Phase 2: Structural detection — role="dialog" with cookie keywords ===
    var cookieKeywords = /cookie|consent|gdpr|privacy|datenschutz|rgpd|privacidad/i;
    try {
        var dialogs = document.querySelectorAll('[role="dialog"], [role="alertdialog"], [aria-modal="true"]');
        for (var di = 0; di < dialogs.length; di++) {
            var dialog = dialogs[di];
            var dialogText = (dialog.textContent || '').slice(0, 500);
            if (cookieKeywords.test(dialogText)) {
                // Found a consent dialog — look for accept button inside
                var btns = dialog.querySelectorAll('button, a[role="button"], [role="button"], .btn');
                for (var bi = 0; bi < btns.length; bi++) {
                    var btnText = (btns[bi].textContent || '').trim().toLowerCase();
                    if (!btnText || btnText.length > 60) continue;
                    for (var ai = 0; ai < acceptTexts.length; ai++) {
                        if (btnText === acceptTexts[ai] || btnText.indexOf(acceptTexts[ai]) !== -1) {
                            btns[bi].click && btns[bi].click();
                            btns[bi].dispatchEvent(new MouseEvent('click', {bubbles: true}));
                            return JSON.stringify({
                                ok: true,
                                method: 'dialog',
                                match: btns[bi].textContent.trim().slice(0, 50)
                            });
                        }
                    }
                }
            }
        }
    } catch (e) {}

    // === Phase 3: Fixed/sticky overlays with cookie content ===
    try {
        var allDivs = document.querySelectorAll('div, section, aside');
        for (var oi = 0; oi < allDivs.length; oi++) {
            var overlay = allDivs[oi];
            var style = overlay.style || {};
            var pos = style.position || '';
            if (pos !== 'fixed' && pos !== 'sticky') continue;
            var overlayText = (overlay.textContent || '').slice(0, 500);
            if (!cookieKeywords.test(overlayText)) continue;
            // Found a fixed/sticky consent overlay
            var overlayBtns = overlay.querySelectorAll('button, a[role="button"], [role="button"], .btn');
            for (var obi = 0; obi < overlayBtns.length; obi++) {
                var obText = (overlayBtns[obi].textContent || '').trim().toLowerCase();
                if (!obText || obText.length > 60) continue;
                for (var oai = 0; oai < acceptTexts.length; oai++) {
                    if (obText === acceptTexts[oai] || obText.indexOf(acceptTexts[oai]) !== -1) {
                        overlayBtns[obi].click && overlayBtns[obi].click();
                        overlayBtns[obi].dispatchEvent(new MouseEvent('click', {bubbles: true}));
                        return JSON.stringify({
                            ok: true,
                            method: 'overlay',
                            match: overlayBtns[obi].textContent.trim().slice(0, 50)
                        });
                    }
                }
            }
        }
    } catch (e) {}

    // === Phase 4: Global button text scan (fallback) ===
    var candidates = document.querySelectorAll(
        'button, a[role="button"], [type="submit"], [role="button"], .btn'
    );
    for (var ci = 0; ci < candidates.length; ci++) {
        var text = (candidates[ci].textContent || '').trim().toLowerCase();
        if (!text || text.length > 60) continue;
        for (var ti = 0; ti < acceptTexts.length; ti++) {
            if (text === acceptTexts[ti] || text.indexOf(acceptTexts[ti]) !== -1) {
                candidates[ci].click && candidates[ci].click();
                candidates[ci].dispatchEvent(new MouseEvent('click', {bubbles: true}));
                return JSON.stringify({
                    ok: true,
                    method: 'text',
                    match: candidates[ci].textContent.trim().slice(0, 50)
                });
            }
        }
    }

    return JSON.stringify({ok: false, reason: 'no consent dialog found'});
};
