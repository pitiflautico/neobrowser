// Page-state digest: the evidence behind every verified action.
//
// Returns `{url, title, hash}` where hash is `elements|text_hash|controls_hash`. Two
// digests differing means the page changed; identical means it did not; empty means we
// could not observe it (which the caller reports as `uncertain`, never as success).
//
// Two properties are load-bearing and easy to break:
//
//   1. NO SECRETS. This digest lands in logs and shareable evidence bundles. Field values
//      are hashed with a per-session salt, never emitted. Password fields contribute only
//      empty-vs-filled — not even a salted hash, because a hash of a password in a log is
//      still a hash of a password. The documented cost is that swapping one non-empty
//      password for another is not detectable.
//   2. IT MUST CROSS SHADOW BOUNDARIES. `querySelectorAll` stops at a shadow root, so a
//      digest built from it alone cannot see a control inside a web component — and a
//      successful fill in there would be reported as `uncertain`.
//
// __SALT__ is a 32-bit integer supplied per session.
(function () {
  var b = document.body;
  if (!b) {
    return JSON.stringify({ url: location.href, title: document.title || '', hash: '0|0|0' });
  }

  function fnv(s, seed) {
    var h = seed >>> 0;
    for (var k = 0; k < s.length; k++) {
      h ^= s.charCodeAt(k);
      h = (h * 16777619) >>> 0;
    }
    return h >>> 0;
  }

  var count = b.querySelectorAll('*').length;

  // Visible text, including shadow content: `innerText` on a host excludes its shadow
  // root's text, so a component that renders all the visible copy would be invisible here.
  var text = b.innerText || '';
  (function shadowText(root, depth) {
    if (depth > 8 || text.length > 200000) return;
    var all;
    try {
      all = root.querySelectorAll('*');
    } catch (e) {
      return;
    }
    for (var k = 0; k < all.length; k++) {
      if (all[k].shadowRoot) {
        try {
          text += '|' + (all[k].shadowRoot.textContent || '');
        } catch (e) {}
        shadowText(all[k].shadowRoot, depth + 1);
      }
    }
  })(b, 0);

  var SEL = 'a,button,input,select,textarea,form,[role],[aria-expanded],[aria-checked]';
  var els = [];
  (function collect(root, depth) {
    if (depth > 8 || els.length > 400) return; // bounded: deep trees must terminate
    var found;
    try {
      found = root.querySelectorAll(SEL);
    } catch (e) {
      return;
    }
    for (var k = 0; k < found.length && els.length <= 400; k++) els.push(found[k]);
    var all;
    try {
      all = root.querySelectorAll('*');
    } catch (e) {
      return;
    }
    for (var k = 0; k < all.length; k++) {
      if (all[k].shadowRoot) collect(all[k].shadowRoot, depth + 1);
    }
  })(b, 0);

  var sig = '';
  for (var i = 0; i < els.length && i < 400; i++) {
    var e = els[i];
    var vh = '';
    if (e.value !== undefined && e.value !== null) {
      if (e.type === 'password') {
        vh = String(e.value).length > 0 ? 'P1' : 'P0';
      } else {
        vh = fnv(String(e.value), __SALT__).toString(16);
      }
    }
    sig +=
      e.tagName +
      '#' + (e.id || '') +
      '@' + (e.getAttribute('role') || '') +
      (e.disabled ? 'D' : '') +
      (e.checked ? 'C' : '') +
      (e.getAttribute('aria-expanded') || '') +
      ':' + vh + ';';
  }

  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    // The text is HASHED, not measured: a length-only component misses a same-length edit
    // ("step 2" -> "step 3"), reporting a real change as no change.
    hash: count + '|' + fnv(text, __SALT__).toString(16) + '|' + fnv(sig, 2166136261).toString(16),
  });
})()
