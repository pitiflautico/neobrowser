"""
T1 PoC: open tab, navigate to example.com, read title, ping, close.

Requires Chrome running with remote debugging enabled.
Usage: python3 poc_t1_chrome_tab.py <port>
       python3 poc_t1_chrome_tab.py 9222

If you need to start Chrome first, run T0 PoC or:
  /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
    --headless=new --remote-debugging-port=9222 --no-sandbox &
"""
from __future__ import annotations

import sys
import os

# Make sure tools/ is on the path when run from this directory
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', '..', '..'))

from tools.v4.chrome_tab import ChromeTab


def main() -> None:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9222
    print(f"[T1 PoC] Using Chrome on port {port}")

    print("[T1 PoC] Opening new tab…")
    tab = ChromeTab.open(port)
    print(f"[T1 PoC] Opened tab id={tab._tab_id}")

    print("[T1 PoC] Navigating to example.com…")
    tab.navigate("https://example.com", wait_s=2.0)

    title = tab.js("return document.title")
    print(f"[T1 PoC] Page title: {title!r}")

    last = tab.wait_last("h1", timeout_s=5.0)
    print(f"[T1 PoC] Last <h1> innerText: {last!r}")

    alive = tab.ping()
    print(f"[T1 PoC] ping() → {alive}")

    tab.close()
    print("[T1 PoC] Tab closed. Done.")


if __name__ == "__main__":
    main()
