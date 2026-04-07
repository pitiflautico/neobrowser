// DOM Tree — extracts the full DOM as a JSON tree for AI consumption.
// Loaded AFTER bootstrap.js (needs document). Called from Rust via V8.

globalThis.__neo_dom_tree = function(maxDepth) {
    if (maxDepth === undefined || maxDepth === null) maxDepth = 50;

    const SKIP = { SCRIPT:1, STYLE:1, SVG:1, NOSCRIPT:1, TEMPLATE:1 };
    const DIRECT_ATTRS = ['id','class','name','href','src','type','placeholder','value','role','aria-label'];

    function buildNode(el, depth) {
        if (!el || !el.tagName) return null;
        var tag = el.tagName.toLowerCase();
        if (SKIP[el.tagName]) return null;
        if (depth > maxDepth) return null;

        var node = { tag: tag };

        // Direct properties (promoted from attrs for easy access)
        for (var i = 0; i < DIRECT_ATTRS.length; i++) {
            var name = DIRECT_ATTRS[i];
            var val = el.getAttribute ? el.getAttribute(name) : null;
            if (val != null && val !== '') {
                // Use camelCase key for aria-label
                var key = name === 'aria-label' ? 'ariaLabel' : name;
                node[key] = val;
            }
        }

        // Other attributes (not in DIRECT_ATTRS)
        var attrs = {};
        var hasAttrs = false;
        if (el.attributes) {
            for (var j = 0; j < el.attributes.length; j++) {
                var attr = el.attributes[j];
                var aname = attr.name || attr.nodeName;
                if (DIRECT_ATTRS.indexOf(aname) === -1 && aname !== 'style') {
                    attrs[aname] = attr.value || attr.nodeValue || '';
                    hasAttrs = true;
                }
            }
        }
        if (hasAttrs) node.attrs = attrs;

        // Text content (only direct text nodes, not nested)
        var textParts = [];
        if (el.childNodes) {
            for (var k = 0; k < el.childNodes.length; k++) {
                var child = el.childNodes[k];
                if (child.nodeType === 3) { // TEXT_NODE
                    var t = child.textContent;
                    if (t) {
                        var trimmed = t.trim();
                        if (trimmed) textParts.push(trimmed);
                    }
                }
            }
        }
        if (textParts.length > 0) {
            var text = textParts.join(' ');
            if (text.length > 300) text = text.slice(0, 300);
            node.text = text;
        }

        // Children (element nodes only)
        var children = [];
        if (el.children && depth < maxDepth) {
            for (var c = 0; c < el.children.length; c++) {
                var childNode = buildNode(el.children[c], depth + 1);
                if (childNode) children.push(childNode);
            }
        }
        if (children.length > 0) node.children = children;

        return node;
    }

    // Start from <html> or <body> depending on what exists
    var root = document.documentElement || document.body;
    if (!root) return JSON.stringify({tag:'html',children:[]});

    var tree = buildNode(root, 0);
    return JSON.stringify(tree || {tag:'html',children:[]});
};
