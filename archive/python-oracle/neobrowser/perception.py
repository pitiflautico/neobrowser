"""
Perception helpers so an AI model is never blind or stuck:

- click_outcome(): did a click navigate / open a modal / update the page / do nothing?
- CLICK_SNAPSHOT_JS: cheap before/after DOM snapshot feeding click_outcome.
- DISMISS_OVERLAY_JS: detect and dismiss cookie/GDPR/newsletter overlays.
- classify_page_state(): label a page login_required / captcha / rate_limited / error.

The click-outcome and page-state ideas are ported from the v3 monolith into this
standalone, unit-testable module.
"""
from __future__ import annotations

import re

# ---------------------------------------------------------------------------
# Click outcome
# ---------------------------------------------------------------------------

# Cheap snapshot of the signals that reveal what a click did. Returned as JSON.
CLICK_SNAPSHOT_JS = """return JSON.stringify({
    url: location.href,
    modals: document.querySelectorAll('[role=dialog],[aria-modal=true],.modal,.popup').length,
    bodyLen: (document.body ? document.body.innerText.length : 0)
});"""


def click_outcome(before: dict, after: dict) -> tuple[str, dict]:
    """
    Derive what a click did from before/after snapshots.

    Returns (outcome, extra) where outcome is one of:
    navigated | modal_opened | page_updated | no_change.
    """
    extra: dict = {}
    if before.get("url", "") != after.get("url", ""):
        extra["new_url"] = after.get("url", "")
        return "navigated", extra
    if after.get("modals", 0) > before.get("modals", 0):
        return "modal_opened", extra
    if abs(after.get("bodyLen", 0) - before.get("bodyLen", 0)) > 500:
        return "page_updated", extra
    return "no_change", extra


# ---------------------------------------------------------------------------
# Overlay dismissal
# ---------------------------------------------------------------------------

DISMISS_OVERLAY_JS = """return (function(force){
    const ACCEPT = ['accept all','accept','agree','i agree','got it','ok','allow all','allow',
        'aceptar','acepto','permitir','entendido','continuar','cerrar'];
    const CLOSE = ['close','dismiss','no thanks','skip','×','✕','✗','x'];
    const click = (el) => { try { el.scrollIntoView(); el.click(); return true; } catch(e){ return false; } };
    const findBtn = (texts, root) => {
        const btns = Array.from((root||document).querySelectorAll(
            'button,a,[role=button],[class*=accept],[class*=agree],[class*=consent],[class*=cookie]'));
        for (const t of texts) {
            const b = btns.find(x => { const s=(x.innerText||'').trim().toLowerCase(); return s===t || s.startsWith(t); });
            if (b) return b;
        }
        return null;
    };
    const overlays = Array.from(document.querySelectorAll('*')).filter(e => {
        const s = getComputedStyle(e);
        return (s.position==='fixed'||s.position==='sticky') && parseInt(s.zIndex||0) > 50
            && e.offsetHeight > 40 && e.offsetWidth > 100;
    });
    if (!overlays.length) return JSON.stringify({dismissed:false, reason:'no overlay detected'});
    for (const o of overlays) { const b = findBtn(ACCEPT, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'accept', text:(b.innerText||'').trim().slice(0,30)}); }
    for (const o of overlays) { const b = findBtn(CLOSE, o); if (b && click(b))
        return JSON.stringify({dismissed:true, method:'close', text:(b.innerText||'').trim().slice(0,30)}); }
    if (force) {
        document.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape', bubbles:true}));
        const bd = document.querySelector('[class*=backdrop],[class*=overlay],[class*=mask]');
        if (bd) click(bd);
        return JSON.stringify({dismissed:true, method:'escape_backdrop'});
    }
    return JSON.stringify({dismissed:false, reason:'no dismiss button found, try force=true'});
})(FORCE);"""


# ---------------------------------------------------------------------------
# Page-state classification
# ---------------------------------------------------------------------------

_STATE_PATTERNS = {
    "login_required": [r"(sign in|log ?in|iniciar sesión|inicia sesión|ingresar)", r"(password|contraseña)"],
    "captcha":        [r"(captcha|i'?m not a robot|are you (a )?human|cloudflare|verify you are)", r"(verify|verificar|robot|challenge)"],
    "rate_limited":   [r"(rate limit|too many requests|slow down|unusual traffic)", r"(429|retry|try again)"],
    "error":          [r"(404|not found|error 5\d\d|access denied|forbidden)", r"(page|página|server)"],
}


def classify_page_state(text: str) -> str | None:
    """
    Return a semantic label for a page from its visible text, or None if it
    looks like a normal, accessible page. Both patterns in a group must match.
    """
    lower = (text or "").lower()[:3000]
    for state, patterns in _STATE_PATTERNS.items():
        if all(re.search(p, lower) for p in patterns):
            return state
    return None
