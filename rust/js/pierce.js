// Reach an element inside shadow DOM or a same-origin iframe.
//
// Depth-first from the document, descending into every open `shadowRoot` and every
// reachable `contentDocument`. Returns the path it took, because "found it, three shadow
// roots down inside an iframe" is information the caller needs to act on later.
//
// Only OPEN shadow roots are reachable. A closed root is closed by the component author's
// choice and there is no honest way around it. Cross-origin `contentDocument` access
// throws; it is caught and skipped, and reported separately by `list_frames`, so a missing
// element is explainable rather than mysterious.
//
// Placeholders: __SEL__ (JSON string), __ACTION__ (JSON string), __VALUE__ (JSON string).
(function () {
  var target = null,
    path = [];

  function search(root, trail, depth) {
    if (target || depth > 12) return; // bounded: cycles and deep trees must terminate
    var hit = null;
    try {
      hit = root.querySelector(__SEL__);
    } catch (e) {
      return;
    }
    if (hit) {
      target = hit;
      path = trail.slice();
      return;
    }
    var all;
    try {
      all = root.querySelectorAll('*');
    } catch (e) {
      return;
    }
    for (var i = 0; i < all.length && !target; i++) {
      var el = all[i];
      if (el.shadowRoot) {
        search(el.shadowRoot, trail.concat(['shadow:' + el.tagName.toLowerCase()]), depth + 1);
      }
      if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') {
        var doc = null;
        try {
          doc = el.contentDocument;
        } catch (e) {
          doc = null;
        }
        if (doc) {
          search(
            doc,
            trail.concat(['iframe:' + (el.getAttribute('src') || '(inline)').slice(0, 80)]),
            depth + 1
          );
        }
      }
    }
  }

  search(document, [], 0);
  if (!target) return JSON.stringify({ found: false });

  var out = { found: true, path: path, tag: target.tagName.toLowerCase() };
  var action = __ACTION__;
  if (action === 'read') {
    out.text = (target.innerText || target.textContent || '').slice(0, 4000);
  } else if (action === 'click') {
    target.scrollIntoView({ block: 'center' });
    target.click();
    out.clicked = true;
  } else if (action === 'fill') {
    var v = __VALUE__;
    target.focus();
    var proto =
      target instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    var d = Object.getOwnPropertyDescriptor(proto, 'value');
    // Through the prototype setter, or React and Vue never learn the value changed: the
    // pixels update and the app keeps the old state.
    if (d && d.set) d.set.call(target, v);
    else target.value = v;
    // `composed: true` is required for an event to cross a shadow boundary.
    target.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    target.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
    out.filled = true;
  }
  return JSON.stringify(out);
})()
