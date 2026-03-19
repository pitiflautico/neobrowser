// NeoRender — Smart Prefetch
// Predict what the AI will want next and have it ready.

globalThis.__neo_prefetch_hints = function() {
    const hints = [];
    const pageType = typeof __neo_classify === 'function' ? __neo_classify() : 'unknown';

    if (pageType === 'search_results') {
        // Pre-identify the top 5 result links
        const results = [...document.querySelectorAll('a')]
            .filter(a => {
                const href = a.href;
                return href && !href.includes('google') && !href.includes('#') &&
                       a.closest?.('[class*="result"],[class*="search"],[data-ved]');
            })
            .slice(0, 5);
        hints.push(...results.map(a => ({
            type: 'search_result',
            text: a.textContent?.trim()?.slice(0, 100),
            url: a.href,
        })));

        // Next page link
        const next = document.querySelector('a[aria-label*="Next"],a[aria-label*="Siguiente"],a#pnnext');
        if (next) hints.push({type: 'next_page', url: next.href});
    }

    if (pageType === 'article') {
        // Related articles
        const related = [...document.querySelectorAll('a')]
            .filter(a => a.closest?.('[class*="related"],[class*="more"],[class*="suggest"]'))
            .slice(0, 3);
        hints.push(...related.map(a => ({type: 'related', text: a.textContent?.trim()?.slice(0, 80), url: a.href})));
    }

    if (pageType === 'login') {
        hints.push({type: 'action_required', action: 'fill_form', fields:
            [...document.querySelectorAll('input:not([type="hidden"])')].map(i => i.name || i.type)
        });
    }

    if (pageType === 'form') {
        // Identify required fields
        const required = [...document.querySelectorAll('input[required],select[required],textarea[required]')]
            .map(i => i.name || i.id || i.type);
        if (required.length > 0) {
            hints.push({type: 'required_fields', fields: required});
        }
        // Submit button
        const submit = document.querySelector('button[type="submit"],input[type="submit"]');
        if (submit) {
            hints.push({type: 'submit', text: submit.textContent?.trim() || submit.value || 'Submit'});
        }
    }

    if (pageType === 'data_table') {
        // Pagination
        const pagination = document.querySelector('a[aria-label*="next"],a[rel="next"],.pagination a:last-child');
        if (pagination) hints.push({type: 'next_page', url: pagination.href});
        // Row count
        const rows = document.querySelectorAll('table tbody tr').length;
        if (rows > 0) hints.push({type: 'table_info', rows});
    }

    return JSON.stringify(hints);
};
