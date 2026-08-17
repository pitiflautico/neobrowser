// Web Vitals, navigation timing and the slowest resources.
//
// Read from the Performance and PerformanceObserver APIs rather than computed here: LCP and
// CLS have subtle definitions that Chrome already implements, and reimplementing them would
// produce numbers that disagree with DevTools.
(function () {
  var out = { collected: true };

  var nav = performance.getEntriesByType('navigation')[0];
  if (nav) {
    out.ttfb = Math.round(nav.responseStart - nav.requestStart);
    out.dom_content_loaded = Math.round(nav.domContentLoadedEventEnd - nav.startTime);
    out.load = Math.round(nav.loadEventEnd - nav.startTime);
    out.transfer_size = nav.transferSize || 0;
    out.protocol = nav.nextHopProtocol || '';
  }

  var paints = {};
  performance.getEntriesByType('paint').forEach(function (p) {
    paints[p.name] = Math.round(p.startTime);
  });
  out.first_paint = paints['first-paint'];
  out.first_contentful_paint = paints['first-contentful-paint'];

  // LCP and CLS are only available via the observer buffer, and only on pages that opted
  // into it — hence the guards rather than assuming they exist.
  try {
    var lcp = performance.getEntriesByType('largest-contentful-paint');
    if (lcp && lcp.length) out.largest_contentful_paint = Math.round(lcp[lcp.length - 1].startTime);
  } catch (e) {}
  try {
    var cls = 0;
    performance.getEntriesByType('layout-shift').forEach(function (s) {
      if (!s.hadRecentInput) cls += s.value;
    });
    out.cumulative_layout_shift = Math.round(cls * 1000) / 1000;
  } catch (e) {}

  var res = performance.getEntriesByType('resource');
  out.resource_count = res.length;
  out.slowest_resources = res
    .map(function (r) {
      return { name: String(r.name).slice(0, 200), ms: Math.round(r.duration), type: r.initiatorType };
    })
    .sort(function (a, b) {
      return b.ms - a.ms;
    })
    .slice(0, 8);

  var mem = performance.memory;
  if (mem) {
    out.js_heap_used_mb = Math.round(mem.usedJSHeapSize / 1048576);
    out.js_heap_limit_mb = Math.round(mem.jsHeapSizeLimit / 1048576);
  }
  return JSON.stringify(out);
})()
