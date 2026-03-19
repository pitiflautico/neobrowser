// NeoRender — Delta Updates
// After navigation/interaction, only send what changed (not the entire page).

globalThis.__neo_page_snapshot = null;

globalThis.__neo_take_snapshot = function() {
    __neo_page_snapshot = {
        title: document.title,
        url: location.href,
        text: document.body?.textContent?.trim()?.slice(0, 5000) || '',
        links: document.querySelectorAll('a').length,
        forms: document.querySelectorAll('form').length,
        buttons: document.querySelectorAll('button').length,
        inputs: document.querySelectorAll('input,textarea,select').length,
    };
    return JSON.stringify({ok: true});
};

globalThis.__neo_get_delta = function() {
    if (!__neo_page_snapshot) return JSON.stringify({delta: false, reason: 'no snapshot'});

    const current = {
        title: document.title,
        url: location.href,
        text: document.body?.textContent?.trim()?.slice(0, 5000) || '',
        links: document.querySelectorAll('a').length,
        forms: document.querySelectorAll('form').length,
        buttons: document.querySelectorAll('button').length,
        inputs: document.querySelectorAll('input,textarea,select').length,
    };

    const changes = {};
    let changed = false;
    for (const [key, val] of Object.entries(current)) {
        if (key === 'text') {
            if (val !== __neo_page_snapshot.text) {
                // Find what's new
                const oldWords = new Set(__neo_page_snapshot.text.split(/\s+/));
                const newWords = current.text.split(/\s+/).filter(w => !oldWords.has(w) && w.length > 3);
                if (newWords.length > 0) {
                    changes.new_text = newWords.slice(0, 50).join(' ');
                    changed = true;
                }
            }
        } else if (val !== __neo_page_snapshot[key]) {
            changes[key] = {old: __neo_page_snapshot[key], new: val};
            changed = true;
        }
    }

    // Update snapshot
    __neo_page_snapshot = current;

    return JSON.stringify({delta: changed, changes});
};
