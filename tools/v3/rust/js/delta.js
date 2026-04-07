// NeoRender — Delta Updates
// After navigation/interaction, only send what changed (not the entire page).

globalThis.__neo_page_snapshot = null;

globalThis.__neo_take_snapshot = function() {
    __neo_page_snapshot = {
        title: document.title,
        url: location.href,
        text: document.body?.textContent?.trim()?.slice(0, 5000) || '',
        links: document.querySelectorAll('a').length,
        forms: document.querySelectorAll('form').length,
        buttons: document.querySelectorAll('button').length,
        inputs: document.querySelectorAll('input,textarea,select').length,
    };
    return JSON.stringify({ok: true});
};

globalThis.__neo_get_delta = function() {
    if (!__neo_page_snapshot) return JSON.stringify({delta: false, reason: 'no snapshot'});

    const current = {
        title: document.title,
        url: location.href,
        text: document.body?.textContent?.trim()?.slice(0, 5000) || '',
        links: document.querySelectorAll('a').length,
        forms: document.querySelectorAll('form').length,
        buttons: document.querySelectorAll('button').length,
        inputs: document.querySelectorAll('input,textarea,select').length,
    };

    const changes = {};
    let changed = false;
    for (const [key, val] of Object.entries(current)) {
        if (key === 'text') {
            if (val !== __neo_page_snapshot.text) {
                // Find what's new
                const oldWords = new Set(__neo_page_snapshot.text.split(/\s+/));
                const newWords = current.text.split(/\s+/).filter(w => !oldWords.has(w) && w.length > 3);
                if (newWords.length > 0) {
                    changes.new_text = newWords.slice(0, 50).join(' ');
                    changed = true;
                }
            }
        } else if (val !== __neo_page_snapshot[key]) {
            changes[key] = {old: __neo_page_snapshot[key], new: val};
            changed = true;
        }
    }

    // Update snapshot
    __neo_page_snapshot = current;

    return JSON.stringify({delta: changed, changes});
};

// ─── Delta Engine v2 — fingerprint-based stable diffs ───

// Fingerprint a DOM node — stable identity across mutations
globalThis.__neo_fingerprint = function(node) {
  if (!node || node.nodeType !== 1) return '';
  const tag = node.tagName?.toLowerCase() || '';
  const classes = (node.className || '').toString().split(/\s+/).sort().join('.');
  const id = node.id || '';
  const role = node.getAttribute?.('role') || '';
  const type = node.getAttribute?.('type') || '';
  // Parent context for disambiguation
  const parentTag = node.parentElement?.tagName?.toLowerCase() || '';
  const siblingIndex = node.parentElement ?
    Array.from(node.parentElement.children).indexOf(node) : 0;

  return `${parentTag}>${tag}#${id}.${classes}[${role}][${type}]@${siblingIndex}`;
};

// Stored fingerprints from last v2 delta call
globalThis.__neo_delta_v2_fingerprints = null;

// __neo_delta_v2(previousFingerprints?) — compute efficient delta
// If previousFingerprints is provided (JSON string), use it; otherwise use stored state.
globalThis.__neo_delta_v2 = function(previousFingerprints) {
  const prev = previousFingerprints
    ? JSON.parse(previousFingerprints)
    : (__neo_delta_v2_fingerprints || {});
  const current = {};
  const added = [];
  const removed = [];
  const changed = [];

  // Focus on interactive elements — bounded to avoid OOM
  const interactiveSelectors = 'a, button, input, select, textarea, [role="button"], [onclick], [tabindex]';
  const interactive = document.querySelectorAll(interactiveSelectors);
  // Cap at 2000 elements to prevent OOM on heavy pages
  const limit = Math.min(interactive.length, 2000);

  for (let i = 0; i < limit; i++) {
    const el = interactive[i];
    const fp = __neo_fingerprint(el);
    if (!fp) continue;
    const text = el.textContent?.trim()?.substring(0, 100) || '';
    const value = el.value || '';
    const tag = el.tagName?.toLowerCase() || '';
    current[fp] = { text, value, tag };

    if (!prev[fp]) {
      if (added.length < 50) added.push({ fingerprint: fp, text, tag });
    } else if (prev[fp].text !== text || prev[fp].value !== value) {
      if (changed.length < 50) changed.push({ fingerprint: fp, old_text: prev[fp].text, new_text: text });
    }
  }

  // Find removed (cap at 50)
  for (const fp in prev) {
    if (!current[fp] && removed.length < 50) {
      removed.push({ fingerprint: fp, text: prev[fp].text });
    }
  }

  // Store for next call
  __neo_delta_v2_fingerprints = current;

  return JSON.stringify({
    added,
    removed,
    changed,
    total_interactive: interactive.length,
    fingerprints: current
  });
};
