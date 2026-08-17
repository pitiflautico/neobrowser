#!/usr/bin/env python3
"""Verify every source file is actually committed, not just present locally.

    python3 scripts/check-tracked.py

A file that exists in the working tree but not in git compiles, tests and passes every other
check on the author's machine, and is simply absent for everyone else. Nothing in the normal
verification story looks at the difference, because every tool reads the working tree.

That is not hypothetical. `.gitignore` carried an unanchored `sessions/`, meant for runtime
state. Unanchored, the pattern matches a directory of that name at any depth — so when
`sessions.rs` was split into `rust/src/sessions/`, git silently declined to add four modules.
The branch built and passed 324 tests locally and did not compile at all once pushed. The
sibling pattern `profiles/` was the same mine waiting for the next person to split that module.

So this asks git, rather than the filesystem, and reports three things it can be wrong about:

  - a source file that exists but is untracked
  - a source file that exists but is actively ignored, which is the silent case
  - a tracked file that no longer exists

Run it before pushing, and in CI, where it costs nothing.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Directories whose contents must be committed, and the extensions that matter in each.
GUARDED = {
    "rust/src": (".rs",),
    "rust/tests": (".rs",),
    "rust/js": (".js",),
    "scripts": (".py", ".sh"),
    "docs": (".md",),
}


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args],
                          capture_output=True, text=True).stdout.splitlines()


def main():
    tracked = set(git("ls-files"))
    problems = []

    for directory, exts in GUARDED.items():
        base = os.path.join(ROOT, directory)
        if not os.path.isdir(base):
            continue
        for dirpath, _, filenames in os.walk(base):
            if "target" in dirpath.split(os.sep):
                continue
            for name in filenames:
                if not name.endswith(exts):
                    continue
                rel = os.path.relpath(os.path.join(dirpath, name), ROOT)
                if rel in tracked:
                    continue
                ignored = subprocess.run(
                    ["git", "-C", ROOT, "check-ignore", "-q", rel]).returncode == 0
                problems.append((rel, "IGNORED by .gitignore" if ignored else "untracked"))

    missing = [f for f in tracked
               if f.split("/")[0] in {d.split("/")[0] for d in GUARDED}
               and not os.path.exists(os.path.join(ROOT, f))]

    for rel, why in sorted(problems):
        print(f"  {why:<22} {rel}")
    for rel in sorted(missing):
        print(f"  {'tracked but deleted':<22} {rel}")

    if problems or missing:
        ignored = [p for p in problems if p[1].startswith("IGNORED")]
        print(f"\n{len(problems) + len(missing)} file(s) differ between the working tree and "
              f"git. Anything listed here builds for you and is absent for everyone else.")
        if ignored:
            print("The IGNORED ones are the dangerous kind: `git add` fails silently on them. "
                  "Check .gitignore for an unanchored directory pattern — `foo/` matches at "
                  "any depth, `/foo/` only at the root.")
        return 1

    print(f"Every source file under {', '.join(sorted(GUARDED))} is committed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
