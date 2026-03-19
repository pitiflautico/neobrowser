// NeoRender — Auto-extraction helpers
// Injected into V8 runtime. Called from Rust extract module.

globalThis.__neo_extract_tables = function() {
    const tables = [];
    for (const table of document.querySelectorAll('table')) {
        const headers = [...table.querySelectorAll('th')].map(th => th.textContent?.trim() || '');
        const rows = [...table.querySelectorAll('tr')].map(tr =>
            [...tr.querySelectorAll('td')].map(td => td.textContent?.trim() || '')
        ).filter(r => r.length > 0);
        if (rows.length > 0) tables.push({headers, rows});
    }
    return JSON.stringify(tables);
};

globalThis.__neo_extract_article = function() {
    // Find the main article content
    const article = document.querySelector('article')
        || document.querySelector('[role="main"]')
        || document.querySelector('main')
        || document.body;
    const title = document.querySelector('h1')?.textContent?.trim() || document.title;

    // Extract clean text (skip nav, aside, footer)
    const body = [];
    function walk(el) {
        if (!el || !el.tagName) return;
        const tag = el.tagName.toLowerCase();
        if (['script','style','nav','aside','footer','header','noscript','svg'].includes(tag)) return;
        if (el.getAttribute?.('role') === 'navigation') return;
        for (const child of el.childNodes || []) {
            if (child.nodeType === 3) {
                const t = child.textContent?.trim();
                if (t) body.push(t);
            } else walk(child);
        }
        if (['p','h1','h2','h3','h4','h5','h6','li','br'].includes(tag)) body.push('\n');
    }
    walk(article);

    // Extract meta
    const author = document.querySelector('[name="author"]')?.content
        || document.querySelector('[rel="author"]')?.textContent?.trim()
        || '';
    const date = document.querySelector('time')?.getAttribute('datetime')
        || document.querySelector('[name="date"]')?.content
        || '';

    return JSON.stringify({
        title,
        author,
        date,
        body: body.join(' ').replace(/\n{3,}/g, '\n\n').trim()
    });
};

globalThis.__neo_extract_form_schema = function(selector) {
    const form = selector ? document.querySelector(selector) : document.querySelector('form');
    if (!form) return JSON.stringify(null);

    const fields = [];
    for (const el of form.querySelectorAll('input, select, textarea')) {
        const field = {
            name: el.name || el.id || '',
            type: el.type || el.tagName.toLowerCase(),
            required: el.required || el.hasAttribute?.('required'),
            placeholder: el.placeholder || '',
            value: el.value || '',
        };
        if (el.tagName === 'SELECT') {
            field.options = [...el.querySelectorAll('option')].map(o => ({
                value: o.value,
                text: o.textContent?.trim()
            }));
        }
        if (field.name) fields.push(field);
    }

    return JSON.stringify({
        action: form.action || '',
        method: (form.method || 'GET').toUpperCase(),
        fields
    });
};

globalThis.__neo_extract_structured = function() {
    // JSON-LD
    const jsonld = [];
    for (const script of document.querySelectorAll('script[type="application/ld+json"]')) {
        try { jsonld.push(JSON.parse(script.textContent)); } catch {}
    }
    // Open Graph
    const og = {};
    for (const meta of document.querySelectorAll('meta[property^="og:"]')) {
        og[meta.getAttribute('property')] = meta.content;
    }
    return JSON.stringify({jsonld, og});
};
