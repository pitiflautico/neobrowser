return JSON.stringify((function(count){
    const out = [], seen = new Set();
    document.querySelectorAll('a#video-title, a#video-title-link, a.yt-simple-endpoint#video-title').forEach(function(a){
        if (out.length >= count) return;
        let href = a.href || ''; if (!href.includes('/watch') || seen.has(href)) return; seen.add(href);
        const title = (a.getAttribute('title') || a.textContent || '').trim();
        let channel = '';
        const card = a.closest('ytd-video-renderer, ytd-rich-item-renderer');
        if (card) { const ch = card.querySelector('ytd-channel-name a, #channel-name a'); if (ch) channel = ch.textContent.trim(); }
        out.push({ url: href, title: title, channel: channel, duration: '', description: '' });
    });
    return out;
})(COUNT))
