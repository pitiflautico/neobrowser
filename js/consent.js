// ═══════════════════════════════════════════════════════════════
// CONSENT AUTO-ACCEPT — dismisses cookie/consent dialogs automatically.
// Loaded AFTER browser.js. Called from Rust after every goto().
// ═══════════════════════════════════════════════════════════════

globalThis.__neo_auto_consent = function() {
    // Common accept button patterns (text content, lowercase)
    const acceptTexts = [
        'accept all', 'aceptar todo', 'accepter tout', 'alle akzeptieren',
        'accept all cookies', 'aceptar todas las cookies',
        'accept', 'aceptar', 'accepter', 'akzeptieren',
        'agree', 'i agree', 'acepto', 'ok', 'got it', 'entendido',
        'allow all', 'permitir todo', 'consent', 'consentir',
        'allow all cookies', 'permitir todas las cookies',
    ];

    // Well-known selectors for consent frameworks
    const selectors = [
        // OneTrust
        '#onetrust-accept-btn-handler',
        // CookieBot
        '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll',
        '#CybotCookiebotDialogBodyButtonAccept',
        // Quantcast
        '.qc-cmp2-summary-buttons button:first-child',
        // StackOverflow / Amazon
        '#sp-cc-accept',
        // CookieConsent.js
        '.cc-btn.cc-allow',
        // Google consent (consent.google.com)
        'button[aria-label="Accept all"]',
        'button[aria-label="Aceptar todo"]',
        'button[aria-label="Tout accepter"]',
        'button[aria-label="Alle akzeptieren"]',
        // Common data attributes
        '[data-testid="cookie-accept"]',
        '[data-cookiebanner="accept_button"]',
        '[data-action="accept"]',
        // Common classes
        '.cookie-consent-accept',
        '#cookie-consent-accept',
        '.js-cookie-consent-agree',
        '.cookie-notice-accept',
        // GDPR frameworks
        '.gdpr-accept',
        '#gdpr-consent-accept',
    ];

    // Try selectors first (most reliable)
    for (const sel of selectors) {
        try {
            const el = document.querySelector(sel);
            if (el) {
                el.click?.();
                el.dispatchEvent(new MouseEvent('click', {bubbles: true}));
                return JSON.stringify({ok: true, method: 'selector', match: sel});
            }
        } catch {}
    }

    // Try by button text content
    const candidates = document.querySelectorAll(
        'button, a[role="button"], [type="submit"], [role="button"], .btn'
    );
    for (const btn of candidates) {
        const text = (btn.textContent || '').trim().toLowerCase();
        if (!text || text.length > 60) continue; // skip empty or very long
        for (const accept of acceptTexts) {
            if (text === accept || text.includes(accept)) {
                btn.click?.();
                btn.dispatchEvent(new MouseEvent('click', {bubbles: true}));
                return JSON.stringify({
                    ok: true,
                    method: 'text',
                    match: btn.textContent.trim().slice(0, 50)
                });
            }
        }
    }

    return JSON.stringify({ok: false, reason: 'no consent dialog found'});
};
