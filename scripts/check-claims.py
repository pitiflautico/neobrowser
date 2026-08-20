#!/usr/bin/env python3
"""Verify that the README's factual claims match the built binary.

Documentation drifts silently. This project has already shipped a README stating "~4 MB
static binary" for a 5.5 MB dynamically-linked one, "43 tools" for 67, and a benchmark
conclusion that was simply false. Each was found by hand, late.

So the checkable claims are checked mechanically. Run it after any change that could move
them, and in CI:

    python3 scripts/check-claims.py

Exits non-zero on the first stale claim, naming what the README says and what is true.
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(ROOT, "rust", "target", "release", "neobrowser")
README = os.path.join(ROOT, "README.md")

# The fence marker in untrusted.rs is not an environment variable, and LOG_LEVEL is only
# read by the benchmark harness.
NOT_ENV_VARS = {"NEOBROWSER_UNTRUSTED_PAGE_CONTENT", "NEOBROWSER_LOG_LEVEL"}

# A rebuild can move the binary by a few hundred KB (debug info, toolchain version) without
# the README being wrong, so the size claim has a tolerance rather than an exact match.
# Wide on purpose. The binary is a different size on every target — a Linux ELF and a
# macOS Mach-O built from identical source differ by more than a tight tolerance allows,
# and the README can only quote one number. The claim worth guarding is the order of
# magnitude ("a few MB, not 80"), not a figure that turns CI red on whichever platform
# the README was not measured on.
SIZE_TOLERANCE_MB = 2.5


def fail(label, stated, actual, hint=""):
    print(f"  STALE  {label}")
    print(f"         README says: {stated}")
    print(f"         reality:     {actual}")
    if hint:
        print(f"         {hint}")
    return 1


def ok(label, value):
    print(f"  OK     {label} ({value})")
    return 0


def main():
    if not os.path.exists(BINARY):
        print("build the release binary first: cd rust && cargo build --release", file=sys.stderr)
        return 2
    readme = open(README).read()
    bad = 0

    # --- tool counts ---
    tools = json.loads(subprocess.run([BINARY, "tools"], capture_output=True, text=True).stdout)
    m = re.search(r"\*\*(\d+) tools\*\*", readme)
    stated = int(m.group(1)) if m else None
    bad += ok("tool count", len(tools)) if stated == len(tools) else fail(
        "tool count", stated, len(tools),
        "regenerate docs too: neobrowser tools --markdown > docs/TOOLS.md")

    core_src = open(os.path.join(ROOT, "rust/src/tools/catalogue.rs")).read()
    core = core_src.split("pub const CORE_TOOLS")[1].split("];")[0]
    core_n = len(re.findall(r'^\s+"', core, re.M))
    m = re.search(r"\((\d+) advertised by default\)", readme)
    stated = int(m.group(1)) if m else None
    bad += ok("advertised tools", core_n) if stated == core_n else fail(
        "advertised tools", stated, core_n)

    # --- binary size ---
    size = round(os.path.getsize(BINARY) / 1048576, 1)
    m = re.search(r"~([\d.]+) MB binary", readme)
    stated = float(m.group(1)) if m else None
    within = stated is not None and abs(stated - size) <= SIZE_TOLERANCE_MB
    bad += ok("binary size", f"{size} MB") if within else fail(
        "binary size", f"~{stated} MB", f"{size} MB")

    # --- every env var the code reads is documented ---
    out = subprocess.run(
        ["grep", "-rhoE", "NEOBROWSER_[A-Z_]+", os.path.join(ROOT, "rust/src")],
        capture_output=True, text=True).stdout
    real = {v for v in out.split() if v.startswith("NEOBROWSER_")} - NOT_ENV_VARS
    documented = set(re.findall(r"\|\s*`(NEOBROWSER_[A-Z_]+)`", readme))
    missing = sorted(real - documented)
    extra = sorted(documented - real)
    if missing or extra:
        bad += fail(
            "environment variables",
            f"{len(documented)} documented",
            f"{len(real)} read by the code",
            f"undocumented: {missing}   documented but unused: {extra}")
    else:
        bad += ok("environment variables", f"{len(real)} documented")

    # --- internal links resolve ---
    refs = [r.split("#")[0] for r in re.findall(r"\]\((?!http)([^)]+)\)", readme)]
    broken = [r for r in refs if r and not r.startswith("#")
              and not os.path.exists(os.path.join(ROOT, r))]
    bad += ok("internal links", f"{len(refs)} checked") if not broken else fail(
        "internal links", "all resolve", f"broken: {broken}")

    print()
    if bad:
        print(f"{bad} stale claim(s). Fix the README or the code — do not leave them disagreeing.")
        return 1
    print("Every checkable README claim matches the binary.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
