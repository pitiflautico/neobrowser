return (function(count) {
    const results = [];
    const seen = new Set();
    const durationRe = /^\d{1,2}:\d{2}(:\d{2})?$/;
    const headings = Array.from(document.querySelectorAll('h3'));
    for (const h3 of headings) {
        if (results.length >= count) break;
        const a = h3.closest('a') || h3.parentElement?.querySelector('a');
        const url = a?.href || '';
        if (!url || seen.has(url)) continue;
        seen.add(url);
        let card = h3.parentElement;
        for (let i = 0; i < 8; i++) { if (!card) break; if (card.innerText?.length > 60) break; card = card.parentElement; }
        const lines = (card?.innerText || '').split('\n').map(l => l.trim()).filter(l => l);
        let duration = '', description = '', channel = '';
        const bodyLines = lines.filter(l => l !== h3.innerText && !l.includes('www.') && !l.startsWith('›'));
        for (const line of bodyLines) {
            if (!duration && durationRe.test(line)) { duration = line; continue; }
            if (line.includes('·')) {
                const parts = line.split('·').map(p => p.trim());
                if (parts.length >= 2 && !channel) {
                    const candidate = parts[1] || parts[0] || '';
                    if (candidate.length < 60 && !/\d+\s*(year|month|day|view|ago)/i.test(candidate)) channel = candidate;
                }
                continue;
            }
            if (!description && line.length > 20) description = line;
        }
        results.push({ title: h3.innerText, url, channel, duration, description: description.slice(0, 300) });
    }
    return JSON.stringify(results);
})(COUNT);
