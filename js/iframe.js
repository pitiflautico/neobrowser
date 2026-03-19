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
