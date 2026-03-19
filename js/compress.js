// compress.js — Semantic compression of page content for AI consumption.
// Prioritizes headings and main content, drops nav/footer/aside noise.
// Returns JSON array of text blocks sorted by priority.

globalThis.__neo_compress = function(maxChars) {
    maxChars = maxChars || 2000;

    // 1. Find main content area
    var main = document.querySelector('main, article, [role="main"], #content, .content')
        || document.body;
    if (!main) return '[]';

    // 2. Extract text blocks with context
    var blocks = [];
    var seen = {}; // deduplicate

    function getPriority(el) {
        var tag = (el.tagName || '').toLowerCase();
        if (tag === 'h1') return 12;
        if (tag === 'h2' || tag === 'h3') return 10;
        if (tag === 'p' || tag === 'li' || tag === 'td') return 5;
        if (tag === 'span' || tag === 'div') return 2;
        try {
            if (el.closest && el.closest('main,article,[role="main"]')) return 8;
            if (el.closest && el.closest('nav,footer,aside')) return 1;
        } catch (e) {}
        return 3;
    }

    function walk(el, depth) {
        if (depth > 20) return;
        var tag = (el.tagName || '').toLowerCase();
        if (['script','style','nav','footer','aside','noscript','svg','iframe'].indexOf(tag) !== -1) return;

        // Only collect leaf-ish elements (avoid duplicating parent + child text)
        var children = el.children || [];
        if (children.length === 0 || ['p','li','td','th','h1','h2','h3','h4','h5','h6','span','a','label'].indexOf(tag) !== -1) {
            var text = (el.textContent || '').trim();
            if (text && text.length > 10) {
                var key = text.substring(0, 80);
                if (!seen[key]) {
                    seen[key] = true;
                    blocks.push({
                        tag: tag,
                        text: text.length > 500 ? text.substring(0, 500) + '...' : text,
                        priority: getPriority(el),
                    });
                }
            }
        }

        for (var i = 0; i < children.length; i++) {
            walk(children[i], depth + 1);
        }
    }

    walk(main, 0);

    // 3. Sort by priority (highest first), truncate to maxChars
    blocks.sort(function(a, b) { return b.priority - a.priority; });
    var total = 0;
    var compressed = [];
    for (var i = 0; i < blocks.length; i++) {
        if (total + blocks[i].text.length > maxChars) break;
        compressed.push(blocks[i]);
        total += blocks[i].text.length;
    }

    return JSON.stringify(compressed);
};
