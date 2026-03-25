//! `cookie_consent` tool — detect and auto-handle cookie consent banners.
//!
//! Scans the page for common cookie consent patterns (buttons, selectors,
//! aria labels) and optionally clicks accept/reject to dismiss them.

use serde_json::Value;

use crate::state::McpState;
use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "cookie_consent",
        description: "Detect and auto-handle cookie consent banners. \
                       Can detect, accept, or reject cookie consent dialogs.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["detect", "accept", "reject"],
                    "description": "What to do: 'detect' (just check), 'accept' (click accept), 'reject' (click reject/decline). Default: 'accept'",
                    "default": "accept"
                }
            }
        }),
    }
}

/// JavaScript that scans the DOM for cookie consent elements.
/// Returns a JSON object with detected buttons and their selectors.
const DETECT_JS: &str = r#"
(function() {
    var result = { found: false, banners: [], buttons: [] };

    // --- Known consent selectors ---
    var knownSelectors = [
        '#onetrust-accept-btn-handler',
        '#onetrust-accept-btn',
        '.cc-accept',
        '#accept-cookies',
        '[data-cookiebanner="accept"]',
        '.cookie-consent-accept',
        '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll',
        '#didomi-notice-agree-button',
        '.fc-cta-consent',
        '#ccpa-button',
        '.js-cookie-consent-agree',
        '[data-testid="cookie-policy-dialog-accept-button"]',
        '.cookie-notice__accept',
        '#cookie-accept',
        '#cookies-accept-all',
        '.gdpr-accept',
        '#consent-accept'
    ];

    var knownRejectSelectors = [
        '#onetrust-reject-all-handler',
        '.cc-deny',
        '#reject-cookies',
        '[data-cookiebanner="reject"]',
        '.cookie-consent-reject',
        '#CybotCookiebotDialogBodyButtonDecline',
        '#didomi-notice-disagree-button',
        '.fc-cta-do-not-consent',
        '.cookie-notice__reject',
        '#cookie-reject',
        '#cookies-reject-all',
        '.gdpr-reject',
        '#consent-reject'
    ];

    // --- Accept button text patterns (case-insensitive) ---
    var acceptTexts = [
        'accept', 'accept all', 'accept cookies', 'aceptar', 'aceptar todo',
        'aceptar todas', 'got it', 'agree', 'i agree', 'allow', 'allow all',
        'permitir', 'akzeptieren', 'alle akzeptieren', 'tout accepter',
        'accepter', 'ok', 'accetta', 'accetta tutti'
    ];

    var rejectTexts = [
        'reject', 'reject all', 'decline', 'deny', 'rechazar', 'rechazar todo',
        'refuse', 'refuser', 'tout refuser', 'ablehnen', 'rifiuta',
        'only necessary', 'solo necesarias', 'nur notwendige',
        'manage preferences', 'manage cookies', 'customize'
    ];

    function getSelector(el) {
        if (el.id) return '#' + el.id;
        var path = [];
        while (el && el.nodeType === 1) {
            var s = el.tagName.toLowerCase();
            if (el.id) { path.unshift('#' + el.id); break; }
            var sib = el.parentNode ? el.parentNode.children : [];
            if (sib.length > 1) {
                var idx = Array.prototype.indexOf.call(sib, el) + 1;
                s += ':nth-child(' + idx + ')';
            }
            path.unshift(s);
            el = el.parentNode;
        }
        return path.join(' > ');
    }

    function isVisible(el) {
        if (!el) return false;
        var style = window.getComputedStyle(el);
        return style.display !== 'none' &&
               style.visibility !== 'hidden' &&
               style.opacity !== '0' &&
               el.offsetWidth > 0 &&
               el.offsetHeight > 0;
    }

    function addButton(el, type, matchMethod) {
        if (!isVisible(el)) return;
        var text = (el.textContent || '').trim().substring(0, 100);
        var selector = getSelector(el);
        result.buttons.push({
            type: type,
            text: text,
            selector: selector,
            tag: el.tagName.toLowerCase(),
            matchMethod: matchMethod
        });
        result.found = true;
    }

    // 1. Check known selectors
    var selectorList = knownSelectors;
    for (var i = 0; i < selectorList.length; i++) {
        var el = document.querySelector(selectorList[i]);
        if (el && isVisible(el)) {
            addButton(el, 'accept', 'known_selector: ' + selectorList[i]);
        }
    }
    for (var i = 0; i < knownRejectSelectors.length; i++) {
        var el = document.querySelector(knownRejectSelectors[i]);
        if (el && isVisible(el)) {
            addButton(el, 'reject', 'known_selector: ' + knownRejectSelectors[i]);
        }
    }

    // 2. Scan buttons and links by text content
    var clickables = document.querySelectorAll('button, a, [role="button"], input[type="button"], input[type="submit"]');
    for (var j = 0; j < clickables.length; j++) {
        var btn = clickables[j];
        if (!isVisible(btn)) continue;
        var btnText = (btn.textContent || btn.value || '').trim().toLowerCase();
        if (!btnText || btnText.length > 50) continue;

        for (var k = 0; k < acceptTexts.length; k++) {
            if (btnText === acceptTexts[k] || btnText.indexOf(acceptTexts[k]) !== -1) {
                addButton(btn, 'accept', 'text_match: ' + acceptTexts[k]);
                break;
            }
        }
        for (var k = 0; k < rejectTexts.length; k++) {
            if (btnText === rejectTexts[k] || btnText.indexOf(rejectTexts[k]) !== -1) {
                addButton(btn, 'reject', 'text_match: ' + rejectTexts[k]);
                break;
            }
        }
    }

    // 3. Check aria-labels containing cookie/consent/privacy
    var ariaEls = document.querySelectorAll('[aria-label]');
    for (var m = 0; m < ariaEls.length; m++) {
        var ael = ariaEls[m];
        if (!isVisible(ael)) continue;
        var aria = (ael.getAttribute('aria-label') || '').toLowerCase();
        if (aria.indexOf('cookie') !== -1 || aria.indexOf('consent') !== -1 || aria.indexOf('privacy') !== -1) {
            var aText = (ael.textContent || '').trim().toLowerCase();
            var aType = 'unknown';
            for (var k = 0; k < acceptTexts.length; k++) {
                if (aText.indexOf(acceptTexts[k]) !== -1 || aria.indexOf('accept') !== -1 || aria.indexOf('agree') !== -1 || aria.indexOf('allow') !== -1) {
                    aType = 'accept'; break;
                }
            }
            if (aType === 'unknown') {
                for (var k = 0; k < rejectTexts.length; k++) {
                    if (aText.indexOf(rejectTexts[k]) !== -1 || aria.indexOf('reject') !== -1 || aria.indexOf('decline') !== -1 || aria.indexOf('deny') !== -1) {
                        aType = 'reject'; break;
                    }
                }
            }
            addButton(ael, aType, 'aria_label: ' + aria);
        }
    }

    // 4. Detect banner containers (common patterns)
    var bannerSelectors = [
        '#onetrust-banner-sdk', '#cookie-banner', '.cookie-banner',
        '#cookiebanner', '.cookiebanner', '#cookie-notice', '.cookie-notice',
        '#cookie-consent', '.cookie-consent', '#gdpr-banner', '.gdpr-banner',
        '#CybotCookiebotDialog', '#didomi-popup', '.fc-consent-root',
        '[role="dialog"][aria-label*="cookie" i]',
        '[role="dialog"][aria-label*="consent" i]',
        '[role="dialog"][aria-label*="privacy" i]'
    ];
    for (var b = 0; b < bannerSelectors.length; b++) {
        try {
            var banner = document.querySelector(bannerSelectors[b]);
            if (banner && isVisible(banner)) {
                result.banners.push({
                    selector: bannerSelectors[b],
                    tag: banner.tagName.toLowerCase(),
                    text: (banner.textContent || '').trim().substring(0, 200)
                });
                result.found = true;
            }
        } catch(e) {}
    }

    // Deduplicate buttons by selector
    var seen = {};
    var unique = [];
    for (var u = 0; u < result.buttons.length; u++) {
        if (!seen[result.buttons[u].selector]) {
            seen[result.buttons[u].selector] = true;
            unique.push(result.buttons[u]);
        }
    }
    result.buttons = unique;

    return JSON.stringify(result);
})()
"#;

/// Execute the `cookie_consent` tool.
pub fn call(args: Value, state: &mut McpState) -> Result<Value, McpError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("accept");

    match action {
        "detect" => call_detect(state),
        "accept" => call_handle(state, "accept"),
        "reject" => call_handle(state, "reject"),
        other => Err(McpError::InvalidParams(format!(
            "unknown action: {other}, expected detect|accept|reject"
        ))),
    }
}

/// Detect cookie consent elements without clicking anything.
fn call_detect(state: &mut McpState) -> Result<Value, McpError> {
    let raw = state.engine.eval(DETECT_JS)?;
    let detection: Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({
        "found": false, "error": "failed to parse detection result"
    }));

    Ok(serde_json::json!({
        "ok": true,
        "action": "detect",
        "consent": detection,
    }))
}

/// Detect and then click the first matching accept/reject button.
fn call_handle(state: &mut McpState, target_type: &str) -> Result<Value, McpError> {
    // Step 1: Detect consent elements.
    let raw = state.engine.eval(DETECT_JS)?;
    let detection: Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({
        "found": false, "buttons": []
    }));

    let found = detection.get("found").and_then(|v| v.as_bool()).unwrap_or(false);

    if !found {
        return Ok(serde_json::json!({
            "ok": true,
            "action": target_type,
            "consent_found": false,
            "clicked": false,
            "message": "No cookie consent banner detected on this page.",
        }));
    }

    // Step 2: Find the best button to click.
    let buttons = detection
        .get("buttons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let target_button = buttons.iter().find(|b| {
        b.get("type")
            .and_then(|v| v.as_str())
            .map(|t| t == target_type)
            .unwrap_or(false)
    });

    let Some(button) = target_button else {
        return Ok(serde_json::json!({
            "ok": true,
            "action": target_type,
            "consent_found": true,
            "clicked": false,
            "message": format!("Cookie consent detected but no '{target_type}' button found."),
            "consent": detection,
        }));
    };

    let selector = button
        .get("selector")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let button_text = button
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if selector.is_empty() {
        return Ok(serde_json::json!({
            "ok": true,
            "action": target_type,
            "consent_found": true,
            "clicked": false,
            "message": "Found consent button but could not determine selector.",
            "consent": detection,
        }));
    }

    // Step 3: Click the button.
    let click_result = state.engine.click(selector);

    match click_result {
        Ok(result) => {
            // Wait briefly for the banner to disappear.
            std::thread::sleep(std::time::Duration::from_millis(500));

            Ok(serde_json::json!({
                "ok": true,
                "action": target_type,
                "consent_found": true,
                "clicked": true,
                "button_text": button_text,
                "button_selector": selector,
                "click_result": serde_json::to_value(result).ok(),
                "consent": detection,
            }))
        }
        Err(e) => Ok(serde_json::json!({
            "ok": false,
            "action": target_type,
            "consent_found": true,
            "clicked": false,
            "error": format!("Failed to click consent button: {e}"),
            "button_text": button_text,
            "button_selector": selector,
            "consent": detection,
        })),
    }
}
