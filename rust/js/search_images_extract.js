return (function(count) {
    const results = [];
    const seen = new Set();
    const scriptText = Array.from(document.querySelectorAll('script')).map(s => s.text).join('\n');
    const imgPattern = /https?:\/\/(?!encrypted-tbn)[\w.\-/%?=&+@#!:,;~]+\.(?:jpg|jpeg|png|webp)(?:[?&][^"'\s<>\\]{0,120})?/gi;
    const rawUrls = [...new Set(scriptText.match(imgPattern) || [])];
    const sourcePairs = Array.from(document.querySelectorAll('a[href^="http"]'))
        .filter(a => !a.href.includes('google.com'))
        .map(a => ({href: a.href, text: a.innerText?.trim() || ''}));
    const imgMeta = {};
    Array.from(document.querySelectorAll('img[alt], img[title]')).forEach(img => {
        const key = (img.src || '').split('?')[0];
        if (key) imgMeta[key] = img.alt || img.title || '';
    });
    const filtered = rawUrls
        .filter(u => !u.includes('gstatic.com') && !u.includes('google.com') && !u.includes('googleapis.com') && u.length > 30)
        .slice(0, count * 3);
    for (const imgUrl of filtered) {
        if (seen.has(imgUrl)) continue;
        seen.add(imgUrl);
        let host = '';
        try { host = new URL(imgUrl).hostname.replace(/^www\./, ''); } catch(e) {}
        const sourcePair = sourcePairs.find(p => {
            try { const ph = new URL(p.href).hostname.replace(/^www\./, ''); return ph.includes(host) || host.includes(ph); }
            catch { return false; }
        });
        results.push({
            image_url: imgUrl,
            source_url: sourcePair?.href || '',
            source_host: host,
            title: imgMeta[imgUrl.split('?')[0]] || sourcePair?.text?.split('\n')[0] || '',
            description: sourcePair?.text || '',
        });
        if (results.length >= count) break;
    }
    return JSON.stringify(results);
})(COUNT);
