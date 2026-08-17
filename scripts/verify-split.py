#!/usr/bin/env python3
"""Verify a module split preserved every line of the original, exactly once.

    python3 scripts/verify-split.py <original.rs> <new/dir>/*.rs

Use this whenever a file is broken into a directory of modules. It is not a CI check —
it needs the pre-split original, so it belongs in the refactor itself.

Weaker checks kept missing things. `cargo build` accepts a lost #[derive]. The tool-catalogue
hash cannot see it. The gap checker only covers the ranges you remembered to declare — and
the hand-written `keep` ranges for mod.rs are exactly where lines went missing three times
running. So this compares multisets of non-trivial lines and reports what was lost or
duplicated, regardless of how the split was expressed.
"""
import sys
from collections import Counter

def norm(lines):
    """Ignore blank lines and bare delimiters — they legitimately move around.

    Visibility prefixes are stripped, because widening a helper to `pub(super)` is the
    expected consequence of a split (the helper now has a sibling caller) and is not a lost
    line. Everything else must match exactly.
    """
    out = Counter()
    for l in lines:
        t = l.strip()
        for vis in ('pub(super) ', 'pub(crate) ', 'pub '):
            if t.startswith(vis):
                t = t[len(vis):]
                break
        if t and t not in ('{', '}', '};', ')', '],', '})', '});', '//!'):
            out[t] += 1
    return out

# Visibility is already stripped by norm(), so these are the bare forms.
ADDED_OK = ('//!', 'mod ', 'use ')

def check(original, outputs, allow_added_prefixes=ADDED_OK):
    o = norm(open(original).read().split('\n'))
    n = Counter()
    for f in outputs:
        n += norm(open(f).read().split('\n'))
    lost = [(l, c - n[l]) for l, c in o.items() if n[l] < c]
    added = [(l, n[l] - o[l]) for l in n if n[l] > o.get(l, 0)
             and not l.startswith(allow_added_prefixes)]
    for l, c in sorted(lost):
        print(f"  PERDIDA x{c}: {l[:100]}")
    for l, c in sorted(added):
        print(f"  DUPLICADA x{c}: {l[:100]}")
    ok = not lost and not added
    print(f"  {'OK  ' if ok else 'FALLO'} {original}: {sum(o.values())} líneas significativas, "
          f"{len(lost)} perdidas, {len(added)} duplicadas")
    return ok


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    sys.exit(0 if check(sys.argv[1], sys.argv[2:]) else 1)
