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
    // Comprehensive structured data extraction — auto-detects all available data.
    // Each sub-extractor is wrapped in try/catch to be resilient.

    function _jsonLd() {
        var scripts = document.querySelectorAll('script[type="application/ld+json"]');
        var out = [];
        for (var i = 0; i < scripts.length && i < 20; i++) {
            try { out.push(JSON.parse(scripts[i].textContent)); } catch(e) {}
        }
        return out;
    }

    function _microdata() {
        var items = document.querySelectorAll('[itemscope]');
        var out = [];
        for (var i = 0; i < items.length && i < 20; i++) {
            var item = items[i];
            var props = {};
            var propEls = item.querySelectorAll('[itemprop]');
            for (var j = 0; j < propEls.length && j < 30; j++) {
                var p = propEls[j];
                var key = p.getAttribute('itemprop') || '';
                if (!key) continue;
                props[key] = p.content || p.href || p.src || (p.textContent || '').trim().substring(0, 200);
            }
            out.push({
                type: item.getAttribute('itemtype') || '',
                properties: props
            });
        }
        return out;
    }

    function _tables() {
        var tables = document.querySelectorAll('table');
        var out = [];
        for (var i = 0; i < tables.length && i < 10; i++) {
            var table = tables[i];
            var headers = [];
            var ths = table.querySelectorAll('th');
            for (var h = 0; h < ths.length; h++) headers.push((ths[h].textContent || '').trim());
            if (headers.length === 0) continue;
            var trs = table.querySelectorAll('tbody tr');
            if (trs.length === 0) trs = table.querySelectorAll('tr');
            var rows = [];
            for (var r = 0; r < trs.length && r < 50; r++) {
                var tds = trs[r].querySelectorAll('td');
                if (tds.length === 0) continue;
                var row = [];
                for (var c = 0; c < tds.length; c++) row.push((tds[c].textContent || '').trim());
                rows.push(row);
            }
            if (rows.length > 0) {
                var totalTrs = table.querySelectorAll('tbody tr');
                out.push({ headers: headers, rows: rows, total_rows: totalTrs.length || rows.length });
            }
        }
        return out;
    }

    function _products() {
        var containers = document.querySelectorAll('[class*="product"], [class*="card"], [data-product], [itemtype*="Product"]');
        if (containers.length < 2) return [];
        var out = [];
        for (var i = 0; i < containers.length && i < 30; i++) {
            var el = containers[i];
            var img = el.querySelector('img');
            var link = el.querySelector('a[href]');
            var priceEl = el.querySelector('[class*="price"], [data-price], [itemprop="price"]');
            var titleEl = el.querySelector('h2, h3, h4, [class*="title"], [class*="name"], [itemprop="name"]') || link;
            var title = titleEl ? (titleEl.textContent || '').trim().substring(0, 100) : '';
            if (!title) continue;
            out.push({
                title: title,
                price: priceEl ? (priceEl.textContent || '').trim() : null,
                image: img ? (img.src || null) : null,
                url: link ? (link.href || null) : null
            });
        }
        return out;
    }

    function _articles() {
        var article = document.querySelector('article, [role="article"], .article, .post, .entry-content, main');
        if (!article) return null;
        var headings = [];
        var hs = article.querySelectorAll('h2, h3');
        for (var i = 0; i < hs.length && i < 20; i++) headings.push((hs[i].textContent || '').trim());
        return {
            title: (document.querySelector('h1') || {textContent: document.title || ''}).textContent.trim(),
            author: (document.querySelector('[rel="author"], .author, [itemprop="author"]') || {}).textContent?.trim() || null,
            date: (document.querySelector('time, [datetime], [itemprop="datePublished"]') || {}).getAttribute?.('datetime')
                || (document.querySelector('time, [itemprop="datePublished"]') || {}).textContent?.trim() || null,
            content_length: (article.textContent || '').length,
            headings: headings,
            links_count: article.querySelectorAll('a[href]').length,
            images_count: article.querySelectorAll('img').length
        };
    }

    function _searchResults() {
        var possible = document.querySelectorAll('[class*="result"], [class*="search"] li, [class*="search"] article');
        if (possible.length < 3) return [];
        var out = [];
        for (var i = 0; i < possible.length && i < 20; i++) {
            var el = possible[i];
            var link = el.querySelector('a[href]');
            if (!link) continue;
            var snippet = el.querySelector('p, span, [class*="snippet"], [class*="desc"]');
            out.push({
                title: (link.textContent || '').trim().substring(0, 100),
                url: link.href || '',
                snippet: snippet ? (snippet.textContent || '').trim().substring(0, 200) : ''
            });
        }
        return out;
    }

    function _pricing() {
        var els = document.querySelectorAll('[class*="price"], [data-price], [itemprop="price"], [itemprop="offers"]');
        if (els.length === 0) return [];
        var out = [];
        for (var i = 0; i < els.length && i < 20; i++) {
            var el = els[i];
            var ctx = el.closest('[itemscope], [class*="product"], [class*="item"]');
            var ctxTitle = ctx ? ctx.querySelector('h2, h3, [class*="title"]') : null;
            out.push({
                text: (el.textContent || '').trim().substring(0, 50),
                value: el.getAttribute('content') || (el.dataset ? el.dataset.price : null) || null,
                currency: el.dataset ? el.dataset.currency : null,
                context: ctxTitle ? (ctxTitle.textContent || '').trim().substring(0, 80) : ''
            });
        }
        return out;
    }

    function _navigation() {
        var nav = document.querySelector('nav, [role="navigation"]');
        if (!nav) return null;
        var links = nav.querySelectorAll('a[href]');
        var out = [];
        for (var i = 0; i < links.length && i < 30; i++) {
            out.push({
                text: (links[i].textContent || '').trim().substring(0, 50),
                url: links[i].href || ''
            });
        }
        return out;
    }

    function _metadata() {
        var q = function(sel) { return document.querySelector(sel); };
        return {
            title: document.title || '',
            description: (q('meta[name="description"]') || {}).content || '',
            canonical: (q('link[rel="canonical"]') || {}).href || '',
            og_title: (q('meta[property="og:title"]') || {}).content || '',
            og_image: (q('meta[property="og:image"]') || {}).content || '',
            og_type: (q('meta[property="og:type"]') || {}).content || '',
            lang: document.documentElement ? (document.documentElement.lang || '') : ''
        };
    }

    // Run all extractors, skip failures
    var sections = {};
    var extractors = {
        json_ld: _jsonLd,
        microdata: _microdata,
        tables: _tables,
        products: _products,
        articles: _articles,
        search_results: _searchResults,
        pricing: _pricing,
        navigation: _navigation,
        metadata: _metadata
    };
    var keys = Object.keys(extractors);
    for (var k = 0; k < keys.length; k++) {
        try {
            var val = extractors[keys[k]]();
            // Skip empty results
            if (val === null || val === undefined) continue;
            if (Array.isArray(val) && val.length === 0) continue;
            if (typeof val === 'object' && !Array.isArray(val) && Object.keys(val).length === 0) continue;
            sections[keys[k]] = val;
        } catch(e) {}
    }
    return JSON.stringify(sections);
};
