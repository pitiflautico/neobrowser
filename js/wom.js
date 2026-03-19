// WOM extraction — runs inside V8 after linkedom parses HTML.
// Walks the live DOM and extracts everything the AI needs in one pass.
// Returns JSON string. Avoids the export_html() + html5ever re-parse cycle.

globalThis.__wom_extract = function() {
    const result = {
        title: document.title || '',
        url: (typeof location !== 'undefined' && location.href) || '',
        text: '',
        links: [],
        forms: [],
        inputs: [],
        buttons: [],
        headings: [],
        images: [],
        meta: {},
    };

    // Extract meta tags
    try {
        for (const meta of document.querySelectorAll('meta[name],meta[property]')) {
            const key = meta.getAttribute('name') || meta.getAttribute('property');
            const val = meta.getAttribute('content');
            if (key && val) result.meta[key] = val;
        }
    } catch {}

    // Walk the DOM
    function walk(el) {
        if (!el || !el.tagName) return;
        const tag = el.tagName.toLowerCase();

        // Skip invisible elements
        if (tag === 'script' || tag === 'style' || tag === 'noscript' ||
            tag === 'svg' || tag === 'template' || tag === 'head') return;

        // Extract visible text from direct text nodes
        if (el.childNodes) {
            for (let i = 0; i < el.childNodes.length; i++) {
                const child = el.childNodes[i];
                if (child.nodeType === 3) { // TEXT_NODE
                    const t = child.textContent;
                    if (t) {
                        const trimmed = t.trim();
                        if (trimmed) {
                            result.text += trimmed;
                            result.text += ' ';
                        }
                    }
                }
            }
        }

        // Block element newlines
        switch (tag) {
            case 'p': case 'div': case 'h1': case 'h2': case 'h3':
            case 'h4': case 'h5': case 'h6': case 'li': case 'tr':
            case 'br': case 'hr': case 'section': case 'article':
            case 'header': case 'footer': case 'nav': case 'main':
            case 'blockquote': case 'figcaption': case 'details':
            case 'summary': case 'dt': case 'dd':
                result.text += '\n';
                break;
        }

        // Links
        if (tag === 'a') {
            const href = el.getAttribute('href');
            if (href && href.charAt(0) !== '#' && !href.startsWith('javascript:')) {
                result.links.push({
                    text: (el.textContent || '').trim().slice(0, 200),
                    href: href
                });
            }
        }

        // Forms
        if (tag === 'form') {
            const fields = [];
            try {
                const formInputs = el.querySelectorAll('input,select,textarea');
                for (let i = 0; i < formInputs.length; i++) {
                    const input = formInputs[i];
                    const name = input.getAttribute('name');
                    if (name) fields.push({
                        name: name,
                        type: input.getAttribute('type') || input.tagName.toLowerCase(),
                        value: input.getAttribute('value') || '',
                        placeholder: input.getAttribute('placeholder') || '',
                    });
                }
            } catch {}
            result.forms.push({
                action: el.getAttribute('action') || '',
                method: (el.getAttribute('method') || 'GET').toUpperCase(),
                fields: fields,
            });
        }

        // Inputs (all, including outside forms)
        if (tag === 'input' || tag === 'textarea' || tag === 'select') {
            const name = el.getAttribute('name') || '';
            const id = el.getAttribute('id') || '';
            if (name || id) {
                result.inputs.push({
                    name: name,
                    id: id,
                    type: el.getAttribute('type') || tag,
                    placeholder: el.getAttribute('placeholder') || '',
                    value: el.getAttribute('value') || '',
                });
            }
        }

        // Buttons
        if (tag === 'button' || (tag === 'input' && (el.getAttribute('type') === 'submit' || el.getAttribute('type') === 'button'))) {
            result.buttons.push({
                text: (el.textContent || el.getAttribute('value') || '').trim().slice(0, 200),
                type: el.getAttribute('type') || '',
                name: el.getAttribute('name') || '',
            });
        }

        // Headings
        if (tag.length === 2 && tag.charAt(0) === 'h' && tag.charAt(1) >= '1' && tag.charAt(1) <= '6') {
            result.headings.push({
                level: parseInt(tag.charAt(1)),
                text: (el.textContent || '').trim().slice(0, 300),
            });
        }

        // Images with alt text
        if (tag === 'img') {
            const alt = el.getAttribute('alt');
            const src = el.getAttribute('src');
            if (alt || src) {
                result.images.push({
                    alt: (alt || '').slice(0, 200),
                    src: (src || '').slice(0, 500),
                });
            }
        }

        // Recurse into children
        if (el.children) {
            for (let i = 0; i < el.children.length; i++) {
                walk(el.children[i]);
            }
        }
    }

    if (document.body) walk(document.body);

    // Clean up text: collapse excessive newlines
    result.text = result.text.replace(/\n{3,}/g, '\n\n').trim();

    return JSON.stringify(result);
};
