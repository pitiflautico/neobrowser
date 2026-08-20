return JSON.stringify((function(limit){
    const out = [], seen = new Set();
    document.querySelectorAll('a h3').forEach(function(h3){
        if (out.length >= limit) return;
        const a = h3.closest('a[href]'); if (!a) return;
        let href = a.href || '';
        if (!href || href.indexOf('https://www.google.') === 0 || seen.has(href)) return;
        seen.add(href);
        let snip = '';
        const c = a.closest('div.g, div.MjjYud, div[data-hveid]');
        if (c) { const s = c.querySelector('.VwiC3b, div[data-sncf], span'); if (s) snip = s.textContent.slice(0,220); }
        out.push({title: h3.textContent.trim(), url: href, snippet: snip.trim()});
    });
    return out;
})(LIMIT))
