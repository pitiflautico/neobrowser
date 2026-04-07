// noise.js — Remove non-essential DOM noise BEFORE WOM extraction.
// Chat widgets, newsletter popups, social share buttons, ad containers.
// Run BEFORE wom.js for cleaner extraction.

globalThis.__neo_remove_noise = function() {
    var removed = 0;

    // 1. Chat widgets (Intercom, Drift, Zendesk, Tawk, Crisp, HubSpot, LiveChat)
    var chatSelectors = [
        '#intercom-container', '#intercom-frame', '.intercom-lightweight-app',
        '#drift-widget', '#drift-frame-controller', '#drift-frame-chat',
        '.drift-frame-controller',
        '#launcher', '#webWidget', '[data-product="web_widget"]', // Zendesk
        '#tawk-bubble-container', '#tawkchat-container', '.tawk-min-container',
        '#crisp-chatbox', '.crisp-client',
        '#hubspot-messages-iframe-container', '#hs-beacon',
        '#chat-widget-container', '.livechat-widget',
        '#tidio-chat', '#tidio-chat-iframe',
        '[id*="zopim"]', '[class*="zopim"]',
        'iframe[src*="intercom"]', 'iframe[src*="drift"]',
        'iframe[src*="zendesk"]', 'iframe[src*="tawk"]',
        'iframe[src*="crisp"]', 'iframe[src*="hubspot"]',
        'iframe[src*="livechat"]', 'iframe[src*="tidio"]',
    ];

    // 2. Newsletter / subscription popups
    var popupSelectors = [
        '[class*="newsletter-popup"]', '[class*="newsletter-modal"]',
        '[class*="subscribe-popup"]', '[class*="subscribe-modal"]',
        '[class*="email-popup"]', '[class*="email-modal"]',
        '[class*="signup-popup"]', '[class*="signup-modal"]',
        '[id*="newsletter-popup"]', '[id*="newsletter-modal"]',
        '[id*="subscribe-popup"]', '[id*="subscribe-modal"]',
        '[class*="exit-intent"]', '[class*="exitintent"]',
        '[class*="popup-overlay"]',
    ];

    // 3. Social media share buttons
    var socialSelectors = [
        '.social-share', '.share-buttons', '.sharing-buttons',
        '[class*="social-share"]', '[class*="share-bar"]',
        '[class*="share-widget"]', '[class*="social-buttons"]',
        '.addthis_toolbox', '.addthis_sharing_toolbox',
        '.sharethis-inline-share-buttons',
        'iframe[src*="facebook.com/plugins"]',
        'iframe[src*="platform.twitter.com"]',
        'iframe[src*="platform.linkedin.com"]',
    ];

    // 4. Ad containers
    var adSelectors = [
        '[class*="ad-container"]', '[class*="ad-wrapper"]', '[class*="ad-slot"]',
        '[class*="ad-banner"]', '[class*="adsbygoogle"]',
        '[id*="ad-container"]', '[id*="ad-wrapper"]', '[id*="ad-slot"]',
        '.advertisement', '.ad-unit', '.ad-block',
        'ins.adsbygoogle', '[data-ad-slot]', '[data-ad-client]',
        'iframe[src*="doubleclick"]', 'iframe[src*="googlesyndication"]',
        'iframe[src*="amazon-adsystem"]',
    ];

    var allSelectors = chatSelectors.concat(popupSelectors, socialSelectors, adSelectors);

    for (var i = 0; i < allSelectors.length; i++) {
        try {
            var els = document.querySelectorAll(allSelectors[i]);
            for (var j = 0; j < els.length; j++) {
                els[j].remove();
                removed++;
            }
        } catch (e) { /* selector may not be valid in all parsers */ }
    }

    return JSON.stringify({ removed: removed });
};
