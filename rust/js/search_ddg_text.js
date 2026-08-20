return JSON.stringify((function(limit){
    const out = [], seen = new Set();
    document.querySelectorAll('.result__body, .result').forEach(function(r){
        if (out.length >= limit) return;
        if ((r.className || '').indexOf('result--ad') !== -1) return;
        const a = r.querySelector('.result__a'); if (!a) return;
        let href = a.href || '';
        if (href.indexOf('/y.js') !== -1 || href.indexOf('ad_domain') !== -1) return;
        try { const u = new URL(href); if (u.searchParams.get('uddg')) href = u.searchParams.get('uddg'); } catch(e){}
        if (!href || seen.has(href)) return;
        seen.add(href);
        const sn = r.querySelector('.result__snippet');
        out.push({title: a.textContent.trim(), url: href, snippet: sn ? sn.textContent.trim() : ''});
    });
    return out;
})(LIMIT))
