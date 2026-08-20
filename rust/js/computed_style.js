// Resolved CSS for one element, its box, and why it is invisible when it is.
//
// Answers "why can't I click this", which a DOM dump cannot: the cascade result is what
// matters and it is not visible in the markup.
//
// Placeholders: __SEL__ (JSON string), __PROPS__ (JSON array of property names).
(function () {
  var e = document.querySelector(__SEL__);
  if (!e) return JSON.stringify({ found: false });

  var cs = getComputedStyle(e);
  var props = __PROPS__;
  var out = { found: true, tag: e.tagName.toLowerCase(), styles: {} };
  props.forEach(function (p) {
    out.styles[p] = cs.getPropertyValue(p);
  });

  var r = e.getBoundingClientRect();
  out.box = {
    x: Math.round(r.x),
    y: Math.round(r.y),
    width: Math.round(r.width),
    height: Math.round(r.height),
  };

  // Answered directly rather than left to be inferred from the properties, because "why is
  // this not visible" is the question that brings people here.
  var reasons = [];
  if (cs.display === 'none') reasons.push('display:none');
  if (cs.visibility === 'hidden') reasons.push('visibility:hidden');
  if (parseFloat(cs.opacity) === 0) reasons.push('opacity:0');
  if (r.width === 0 || r.height === 0) reasons.push('zero-sized box');
  out.hidden_because = reasons;

  return JSON.stringify(out);
})()
