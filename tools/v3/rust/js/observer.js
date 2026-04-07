// Observer — MutationObserver integration + snapshot-based diff fallback.
// Loaded AFTER wom.js, BEFORE browser.js. Provides mutation tracking + page diff.

(function() {
    'use strict';

    // Mutation buffer
    var mutations = [];
    var observerActive = false;

    // Snapshot for diff fallback (used if MutationObserver doesn't fire)
    var lastSnapshot = null;
    var mutationObserverWorks = false;

    // ─── Helper: describe a DOM node compactly ───
    function describeNode(node) {
        if (!node || !node.tagName) return null;
        var desc = { tag: node.tagName.toLowerCase() };
        var id = node.getAttribute ? node.getAttribute('id') : null;
        if (id) desc.id = id;
        var cls = node.getAttribute ? node.getAttribute('class') : null;
        if (cls) desc.class = cls;
        return desc;
    }

    // ─── Helper: describe added/removed nodes ───
    function describeNodeList(nodeList) {
        var result = [];
        for (var i = 0; i < nodeList.length; i++) {
            var node = nodeList[i];
            if (node.nodeType === 1) { // ELEMENT_NODE
                var desc = describeNode(node);
                if (desc) result.push(desc);
            } else if (node.nodeType === 3) { // TEXT_NODE
                var text = (node.textContent || '').trim();
                if (text) result.push({ text: text.slice(0, 100) });
            }
        }
        return result;
    }

    // ─── MutationObserver callback ───
    function handleMutations(mutationList) {
        mutationObserverWorks = true;
        for (var i = 0; i < mutationList.length; i++) {
            var m = mutationList[i];
            var entry = {
                type: m.type,
                target: describeNode(m.target)
            };

            if (m.type === 'childList') {
                if (m.addedNodes.length > 0) entry.added = describeNodeList(m.addedNodes);
                if (m.removedNodes.length > 0) entry.removed = describeNodeList(m.removedNodes);
            } else if (m.type === 'attributes') {
                entry.attr = {
                    name: m.attributeName,
                    old: m.oldValue,
                    new: m.target.getAttribute ? m.target.getAttribute(m.attributeName) : null
                };
            } else if (m.type === 'characterData') {
                entry.text = {
                    old: m.oldValue,
                    new: m.target.textContent || ''
                };
            }

            mutations.push(entry);
        }
    }

    // ─── Try to start MutationObserver ───
    function startObserver() {
        if (observerActive) return;
        try {
            if (typeof MutationObserver !== 'undefined' && document.body) {
                var observer = new MutationObserver(handleMutations);
                observer.observe(document.body, {
                    childList: true,
                    subtree: true,
                    attributes: true,
                    attributeOldValue: true,
                    characterData: true,
                    characterDataOldValue: true
                });
                observerActive = true;
            }
        } catch (e) {
            // MutationObserver not available or broken — fallback to snapshot diff
        }
        // Take initial snapshot regardless (for diff fallback)
        lastSnapshot = takeSnapshot();
    }

    // ─── Snapshot-based diff (fallback when MutationObserver doesn't fire) ───

    function takeSnapshot() {
        if (!document.body) return { nodeCount: 0, textHash: '', structure: '' };
        var nodeCount = 0;
        var texts = [];
        var structure = [];

        function walk(el, depth) {
            if (!el || !el.tagName) return;
            var tag = el.tagName.toLowerCase();
            if (tag === 'script' || tag === 'style') return;
            nodeCount++;

            var id = el.getAttribute ? (el.getAttribute('id') || '') : '';
            structure.push(depth + ':' + tag + (id ? '#' + id : ''));

            // Direct text
            if (el.childNodes) {
                for (var i = 0; i < el.childNodes.length; i++) {
                    if (el.childNodes[i].nodeType === 3) {
                        var t = (el.childNodes[i].textContent || '').trim();
                        if (t) texts.push(t.slice(0, 100));
                    }
                }
            }

            if (el.children) {
                for (var c = 0; c < el.children.length; c++) {
                    walk(el.children[c], depth + 1);
                }
            }
        }
        walk(document.body, 0);

        return {
            nodeCount: nodeCount,
            textHash: texts.join('|').slice(0, 5000),
            structure: structure.join(',').slice(0, 10000)
        };
    }

    function computeSnapshotDiff(oldSnap, newSnap) {
        if (!oldSnap || !newSnap) return null;

        var diff = {
            nodesAdded: Math.max(0, newSnap.nodeCount - oldSnap.nodeCount),
            nodesRemoved: Math.max(0, oldSnap.nodeCount - newSnap.nodeCount),
            attrsChanged: 0, // can't detect from snapshots
            textChanged: oldSnap.textHash !== newSnap.textHash ? 1 : 0,
            structureChanged: oldSnap.structure !== newSnap.structure
        };
        return diff;
    }

    // ─── Public API ───

    // Get all accumulated mutations and clear buffer
    globalThis.__neo_get_mutations = function() {
        // If MutationObserver never fired, try snapshot diff to synthesize mutations
        if (!mutationObserverWorks && lastSnapshot) {
            var newSnap = takeSnapshot();
            var snapDiff = computeSnapshotDiff(lastSnapshot, newSnap);
            lastSnapshot = newSnap;
            if (snapDiff && (snapDiff.nodesAdded > 0 || snapDiff.nodesRemoved > 0 || snapDiff.textChanged)) {
                mutations.push({
                    type: 'snapshot-diff',
                    target: { tag: 'body' },
                    diff: snapDiff
                });
            }
        }

        var result = JSON.stringify(mutations);
        mutations = [];
        return result;
    };

    // Get a summary diff
    globalThis.__neo_get_diff = function() {
        var nodesAdded = 0, nodesRemoved = 0, attrsChanged = 0, textChanged = 0;
        var details = [];

        // If MutationObserver worked, summarize from mutations buffer
        if (mutationObserverWorks || mutations.length > 0) {
            for (var i = 0; i < mutations.length; i++) {
                var m = mutations[i];
                if (m.type === 'childList') {
                    nodesAdded += (m.added ? m.added.length : 0);
                    nodesRemoved += (m.removed ? m.removed.length : 0);
                } else if (m.type === 'attributes') {
                    attrsChanged++;
                } else if (m.type === 'characterData') {
                    textChanged++;
                }
                if (details.length < 20) details.push(m);
            }
        } else {
            // Fallback: snapshot-based diff
            var newSnap = takeSnapshot();
            var snapDiff = computeSnapshotDiff(lastSnapshot, newSnap);
            lastSnapshot = newSnap;
            if (snapDiff) {
                nodesAdded = snapDiff.nodesAdded;
                nodesRemoved = snapDiff.nodesRemoved;
                textChanged = snapDiff.textChanged;
                if (snapDiff.structureChanged) {
                    details.push({ type: 'snapshot-diff', note: 'DOM structure changed (MutationObserver unavailable)' });
                }
            }
        }

        // Clear mutations after reading diff
        mutations = [];

        return JSON.stringify({
            nodesAdded: nodesAdded,
            nodesRemoved: nodesRemoved,
            attrsChanged: attrsChanged,
            textChanged: textChanged,
            details: details
        });
    };

    // Expose mutations array for direct access
    globalThis.__neo_mutations = mutations;

    // Start observing as soon as possible
    if (document.body) {
        startObserver();
    } else {
        // Body not ready yet — try after DOMContentLoaded
        try {
            document.addEventListener('DOMContentLoaded', function() {
                startObserver();
            });
        } catch (e) {}
        // Also try a delayed start as last resort
        if (typeof setTimeout !== 'undefined') {
            setTimeout(function() {
                if (!observerActive) startObserver();
            }, 0);
        }
    }
})();
