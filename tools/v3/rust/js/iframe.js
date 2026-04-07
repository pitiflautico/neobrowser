// NeoRender iframe support — intercepts iframe creation to fetch + parse nested documents.
// Uses __linkedom_parseHTML to create real DOM for iframe content.

(function() {
    const _origCreate = document.createElement.bind(document);
    document.createElement = function(tag, ...args) {
        const el = _origCreate(tag, ...args);
        if (tag.toLowerCase() === 'iframe') {
            // When src is set, fetch and create a mini-document
            const origSetAttr = el.setAttribute.bind(el);
            el.setAttribute = function(name, value) {
                origSetAttr(name, value);
                if (name === 'src' && value) {
                    try {
                        const resp = globalThis.fetch(value);
                        resp.then(r => r.text()).then(html => {
                            const { document: iframeDoc } = __linkedom_parseHTML(html);
                            el.contentDocument = iframeDoc;
                            el.contentWindow = {
                                document: iframeDoc,
                                location: { href: value },
                                parent: globalThis,
                                top: globalThis,
                                postMessage: function(data, origin) {
                                    const event = new MessageEvent('message', { data, origin, source: el.contentWindow });
                                    globalThis.dispatchEvent(event);
                                }
                            };
                            // Execute scripts in iframe
                            for (const script of iframeDoc.querySelectorAll('script')) {
                                if (script.textContent) {
                                    try { eval(script.textContent); } catch {}
                                }
                            }
                        }).catch(() => {});
                    } catch {}
                }
            };
            // Also intercept direct .src assignment
            Object.defineProperty(el, 'src', {
                set(value) { el.setAttribute('src', value); },
                get() { return el.getAttribute('src') || ''; },
                configurable: true,
            });
        }
        return el;
    };
})();

// ─── iframe enumeration & content extraction ───

// __neo_list_frames() — enumerate all iframes with their properties
globalThis.__neo_list_frames = function() {
  const frames = document.querySelectorAll('iframe');
  return JSON.stringify(Array.from(frames).map((frame, i) => {
    let accessible = false;
    let title = '';
    try {
      // Same-origin check (in linkedom, contentDocument is set by our intercept above)
      const doc = frame.contentDocument;
      accessible = !!doc;
      title = doc?.title || '';
    } catch(e) {
      accessible = false; // cross-origin
    }
    return {
      index: i,
      src: frame.src || frame.getAttribute?.('src') || '',
      id: frame.id || null,
      name: frame.name || null,
      sandbox: frame.sandbox?.toString?.() || frame.getAttribute?.('sandbox') || null,
      accessible,
      title,
      width: frame.width || frame.offsetWidth || 0,
      height: frame.height || frame.offsetHeight || 0
    };
  }));
};

// __neo_frame_content(index) — get content from accessible iframe
globalThis.__neo_frame_content = function(index) {
  const frames = document.querySelectorAll('iframe');
  const frame = frames[index];
  if (!frame) return JSON.stringify({ok: false, error: 'frame not found'});
  try {
    const doc = frame.contentDocument;
    if (!doc) return JSON.stringify({ok: false, error: 'not accessible (cross-origin or not loaded)'});
    return JSON.stringify({
      ok: true,
      title: doc.title || '',
      text: doc.body?.textContent?.trim()?.substring(0, 5000) || '',
      links: Array.from(doc.querySelectorAll('a[href]')).slice(0, 20).map(a => ({
        text: a.textContent?.trim()?.substring(0, 50),
        href: a.href
      })),
      forms: Array.from(doc.querySelectorAll('form')).length,
      inputs: Array.from(doc.querySelectorAll('input, textarea, select')).length
    });
  } catch(e) {
    return JSON.stringify({ok: false, error: e.message});
  }
};
