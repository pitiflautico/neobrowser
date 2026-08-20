#!/usr/bin/env python3
"""Verify each item kept the doc comment that was written for it.

    python3 scripts/verify-docs.py <original.rs> <new/dir>/*.rs

scripts/verify-split.py compares multisets of lines, so it proves nothing was lost or
duplicated. It cannot see *misattribution*: a doc comment that moves from function A to
function B preserves every line and still makes the documentation wrong.

That is not hypothetical. Splitting page.rs rotated four doc comments by one function each,
so `box_center` — which measures an element's bounding box — was documented as "Move the
cursor to (tx, ty) over several eased, jittered steps". Every line survived. The crate
compiled. `cargo doc` was clean. The documentation was actively misleading, which is worse
than absent, because a reader has no reason to distrust it.

So this pairs every item with its doc in the original and in the split, and reports any pair
that changed. Rewritten prose is reported too — that is usually intentional during a split
(a module doc gets expanded), so read the output rather than requiring silence.
"""
import re
import sys


def pairs(text):
    """{item_name: doc_text} for every documented top-level item."""
    out, buf = {}, []
    for line in text.split("\n"):
        t = line.strip()
        if t.startswith("///"):
            buf.append(t[3:].strip())
            continue
        if t.startswith("#["):
            continue
        m = re.match(
            r"(?:pub(?:\([^)]*\))? )?(?:async )?(?:unsafe )?"
            r"(?:fn|struct|enum|const|static|trait|type) (\w+)",
            t,
        )
        if m and buf:
            out[m.group(1)] = " ".join(buf)
        if t:
            buf = []
    return out


def check(original, outputs):
    before = pairs(open(original).read())
    after = {}
    for f in outputs:
        after.update(pairs(open(f).read()))

    changed = []
    for name, doc in before.items():
        if name in after and after[name] != doc:
            changed.append((name, doc, after[name]))
    lost = [n for n in before if n not in after]

    for name, was, now in changed:
        print(f"  CHANGED  {name}")
        print(f"     was: {was[:100]}")
        print(f"     now: {now[:100]}")
    if lost:
        print(f"  items no longer documented: {sorted(lost)}")
    ok = not changed and not lost
    print(
        f"  {'OK  ' if ok else 'DIFF'} {original}: {len(before)} documented items, "
        f"{len(changed)} reattributed, {len(lost)} undocumented"
    )
    return ok


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    sys.exit(0 if check(sys.argv[1], sys.argv[2:]) else 1)
