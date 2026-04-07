// DOM Events module — proper click/type/find sequences for NeoRender V8
(function() {
  'use strict';

  // 8-strategy element resolution
  function resolve(target) {
    if (!target) return null;
    // 1. CSS selector
    try { var el = document.querySelector(target); if (el) return el; } catch(e) {}
    // 2. Text content (clickable elements first)
    var clickable = document.querySelectorAll('a, button, [role="button"], input[type="submit"], input[type="button"]');
    for (var i = 0; i < clickable.length; i++) {
      if (clickable[i].textContent && clickable[i].textContent.trim().toLowerCase().indexOf(target.toLowerCase()) !== -1) return clickable[i];
    }
    // 3. All elements text
    var all = document.querySelectorAll('*');
    for (var i = 0; i < Math.min(all.length, 5000); i++) {
      var t = all[i].textContent ? all[i].textContent.trim() : '';
      if (t.toLowerCase() === target.toLowerCase() && t.length < 200) return all[i];
    }
    // 4. aria-label
    try { var el = document.querySelector('[aria-label="' + target + '"]'); if (el) return el; } catch(e) {}
    try { var el = document.querySelector('[aria-label*="' + target + '" i]'); if (el) return el; } catch(e) {}
    // 5. placeholder
    try { var el = document.querySelector('[placeholder*="' + target + '" i]'); if (el) return el; } catch(e) {}
    // 6. name attribute
    try { var el = document.querySelector('[name="' + target + '"]'); if (el) return el; } catch(e) {}
    // 7. title attribute
    try { var el = document.querySelector('[title*="' + target + '" i]'); if (el) return el; } catch(e) {}
    // 8. data-testid
    try { var el = document.querySelector('[data-testid="' + target + '"]'); if (el) return el; } catch(e) {}
    return null;
  }

  function selectorPath(el) {
    if (!el) return '';
    if (el.id) return '#' + el.id;
    var tag = (el.tagName || '').toLowerCase();
    var name = el.getAttribute ? el.getAttribute('name') : '';
    if (name) return tag + '[name="' + name + '"]';
    var cls = (el.className || '').toString().trim().split(/\s+/).slice(0, 2).join('.');
    return cls ? tag + '.' + cls : tag;
  }

  // Proper click sequence
  window.__neo_click = function(target) {
    var el = resolve(target);
    if (!el) return JSON.stringify({ok: false, error: 'not found: ' + target});
    if (el.scrollIntoView) try { el.scrollIntoView({block: 'center'}); } catch(e) {}
    try {
      el.dispatchEvent(new MouseEvent('mouseenter', {bubbles: true}));
      el.dispatchEvent(new MouseEvent('mouseover', {bubbles: true}));
      el.dispatchEvent(new MouseEvent('mousedown', {bubbles: true, button: 0}));
      if (el.focus) el.focus();
      el.dispatchEvent(new MouseEvent('mouseup', {bubbles: true, button: 0}));
      el.dispatchEvent(new MouseEvent('click', {bubbles: true, button: 0}));
    } catch(e) {}
    return JSON.stringify({ok: true, tag: (el.tagName||'').toLowerCase(), href: el.href || null, selector: selectorPath(el)});
  };

  // Proper type sequence
  window.__neo_type = function(target, text, clear) {
    var el = resolve(target);
    if (!el) return JSON.stringify({ok: false, error: 'not found: ' + target});
    if (el.focus) el.focus();
    try { el.dispatchEvent(new FocusEvent('focus', {bubbles: true})); } catch(e) {}
    try { el.dispatchEvent(new FocusEvent('focusin', {bubbles: true})); } catch(e) {}
    if (clear) {
      el.value = '';
      try { el.dispatchEvent(new Event('input', {bubbles: true})); } catch(e) {}
    }
    for (var i = 0; i < text.length; i++) {
      var ch = text[i];
      try {
        el.dispatchEvent(new KeyboardEvent('keydown', {key: ch, bubbles: true}));
        el.dispatchEvent(new KeyboardEvent('keypress', {key: ch, bubbles: true}));
        if ('value' in el) { el.value = (el.value || '') + ch; }
        else if (el.textContent !== undefined) { el.textContent = (el.textContent || '') + ch; }
        el.dispatchEvent(new Event('input', {bubbles: true}));
        el.dispatchEvent(new KeyboardEvent('keyup', {key: ch, bubbles: true}));
      } catch(e) {}
    }
    try { el.dispatchEvent(new Event('change', {bubbles: true})); } catch(e) {}
    return JSON.stringify({ok: true, value: el.value || el.textContent || '', selector: selectorPath(el)});
  };

  // Find element
  window.__neo_find = function(target) {
    var el = resolve(target);
    if (!el) return JSON.stringify({ok: false, error: 'not found: ' + target});
    return JSON.stringify({
      ok: true,
      tag: (el.tagName||'').toLowerCase(),
      text: (el.textContent||'').trim().substring(0, 100),
      selector: selectorPath(el),
      attrs: {
        id: el.id || null,
        name: el.getAttribute ? el.getAttribute('name') : null,
        type: el.getAttribute ? el.getAttribute('type') : null,
        href: el.href || null,
        role: el.getAttribute ? el.getAttribute('role') : null
      }
    });
  };
})();
