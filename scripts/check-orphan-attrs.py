#!/usr/bin/env python3
"""Find attributes separated from the item they were written for.

Rust accepts `#[derive(Debug)]` followed by a doc comment and then an item — it just
applies the derive to the *wrong* item. Splitting a module is where this happens: a
boundary drawn at the `pub enum` line leaves the `#[derive]` above it in the previous
chunk, where it silently attaches to whatever comes next.

This is not hypothetical. Splitting page.rs moved ClickOutcome's
`#[derive(Debug, Clone, PartialEq, Eq)]` onto an unrelated struct, and splitting chrome.rs
dropped ChromeProcess's `#[derive(Debug)]` entirely. Neither broke a single one of the 303
tests, because nothing in the suite Debug-formats those types. The compiler is no help: both
programs are valid.

The signature is an attribute line immediately followed by a doc comment. Legitimate code
puts docs *above* attributes, never between an attribute and its item.

    python3 scripts/check-orphan-attrs.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def main():
    findings = []
    for path in sorted(ROOT.joinpath("rust").rglob("*.rs")):
        if "target" in path.parts:
            continue
        lines = path.read_text().split("\n")
        for i, line in enumerate(lines[:-1]):
            stripped = line.strip()
            if not stripped.startswith("#["):
                continue
            nxt = lines[i + 1].strip()
            # `///` after an attribute means the attribute lost its item. `//!` means a
            # module doc ended up below code, which is a different bug but equally wrong.
            if nxt.startswith("///") or nxt.startswith("//!"):
                findings.append((path.relative_to(ROOT), i + 1, stripped, nxt))

    for rel, line, attr, nxt in findings:
        print(f"{rel}:{line}")
        print(f"  {attr}")
        print(f"  {nxt}   <- doc comment between an attribute and its item")
    if findings:
        print(f"\n{len(findings)} orphaned attribute(s). The attribute is being applied to "
              f"the wrong item, or to nothing.")
        return 1
    print("No orphaned attributes.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
