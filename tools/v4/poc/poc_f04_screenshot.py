"""
tools/v4/poc/poc_f04_screenshot.py

F04 — Screenshot PoC

Validates ChromeTab.screenshot() against a real Chrome tab.

Prerequisites:
  - Chrome running on port 55715
  - tools/v4 importable (run from project root)

Usage:
    python3 tools/v4/poc/poc_f04_screenshot.py
"""
from __future__ import annotations

import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", ".."))

from tools.v4.chrome_tab import ChromeTab

CHROME_PORT = 55715
PNG_SAVE = "/tmp/poc_f04_screenshot.png"
JPEG_SAVE = "/tmp/poc_f04_screenshot.jpg"

PASS = "\033[32m[PASS]\033[0m"
FAIL = "\033[31m[FAIL]\033[0m"
overall_pass = True


def check(label: str, condition: bool, detail: str = "") -> None:
    global overall_pass
    status = PASS if condition else FAIL
    suffix = f"  ({detail})" if detail else ""
    print(f"  {status} {label}{suffix}")
    if not condition:
        overall_pass = False


def main() -> None:
    print("=" * 60)
    print("F04 Screenshot PoC")
    print("=" * 60)

    print("\n[1] Opening tab + navigating to example.com…")
    tab = ChromeTab.open(CHROME_PORT)
    try:
        tab.navigate("https://example.com", wait_s=2.0)

        # ------------------------------------------------------------------ #
        # Check 1: screenshot() returns bytes > 0
        # ------------------------------------------------------------------ #
        print("\n[2] Taking PNG screenshot…")
        png_bytes = tab.screenshot(format="png")
        check("screenshot() returns bytes", isinstance(png_bytes, bytes))
        check("bytes > 0", len(png_bytes) > 0, f"size={len(png_bytes):,}B")

        # ------------------------------------------------------------------ #
        # Check 2: PNG magic header
        # ------------------------------------------------------------------ #
        PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
        check(
            "bytes start with PNG magic header",
            png_bytes[:8] == PNG_MAGIC,
            f"header={png_bytes[:8].hex()}",
        )

        # ------------------------------------------------------------------ #
        # Check 3: screenshot_base64() decodes to same bytes
        # ------------------------------------------------------------------ #
        import base64
        b64 = tab.screenshot_base64(format="png")
        decoded = base64.b64decode(b64)
        check("screenshot_base64() decodes to same bytes", decoded == png_bytes)

        # ------------------------------------------------------------------ #
        # Check 4: screenshot_save() writes PNG to disk
        # ------------------------------------------------------------------ #
        from pathlib import Path
        saved = tab.screenshot_save(PNG_SAVE, format="png")
        check("screenshot_save() writes to disk", Path(PNG_SAVE).exists())
        check(
            "saved file size > 0",
            Path(PNG_SAVE).stat().st_size > 0,
            f"size={Path(PNG_SAVE).stat().st_size:,}B",
        )
        print(f"    Saved PNG: {saved}")

        # ------------------------------------------------------------------ #
        # Check 5: JPEG is smaller than PNG
        # ------------------------------------------------------------------ #
        print("\n[3] Taking JPEG screenshot (quality=60)…")
        jpeg_bytes = tab.screenshot(format="jpeg", quality=60)
        check(
            "format='jpeg' returns smaller bytes than png",
            len(jpeg_bytes) < len(png_bytes),
            f"jpeg={len(jpeg_bytes):,}B  png={len(png_bytes):,}B",
        )
        # JPEG magic: FF D8 FF
        check(
            "JPEG has correct magic bytes",
            jpeg_bytes[:2] == b"\xff\xd8",
            f"header={jpeg_bytes[:4].hex()}",
        )

        tab.screenshot_save(JPEG_SAVE, format="jpeg", quality=60)
        print(f"    Saved JPEG: {JPEG_SAVE}")

    finally:
        tab.close()

    print("\n" + "=" * 60)
    if overall_pass:
        print("\033[32mOVERALL: PASS\033[0m")
    else:
        print("\033[31mOVERALL: FAIL\033[0m")
    print("=" * 60)
    sys.exit(0 if overall_pass else 1)


if __name__ == "__main__":
    main()
