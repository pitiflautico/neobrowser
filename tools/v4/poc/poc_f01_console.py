"""
poc_f01_console.py

F01: Console Log Capture — PoC against a live Chrome instance.

Usage:
    python3 tools/v4/poc/poc_f01_console.py

Requires Chrome running with --remote-debugging-port=55715.
"""
import sys
import time

sys.path.insert(0, ".")
from tools.v4.chrome_tab import ChromeTab

PORT = 55715

print(f"[PoC F01] Connecting to Chrome on port {PORT}...")
tab = ChromeTab.open(PORT)
print(f"  tab_id={tab._tab_id[:16]}...")

# 1. Enable console capture BEFORE navigating
tab.enable_console()
print("  enable_console() called")

# 2. Navigate to page that generates console output.
# data: URL with inline script that calls console.log / warn / error + throws.
DATA_URL = (
    "data:text/html,"
    "<script>"
    "console.log('hello v4');"
    "console.warn('warn test');"
    "console.error('error test');"
    "throw new Error('test error');"
    "</script>"
)
print("  Navigating to data: URL with inline console calls...")
try:
    tab.navigate(DATA_URL, wait_s=1.0)
except Exception as e:
    # Some Chrome configs reject data: URLs — fall back to example.com + JS inject
    print(f"  data: URL navigation raised {e!r} — falling back to example.com + JS inject")
    tab.navigate("https://example.com", wait_s=2.0)
    tab.js("console.log('hello v4'); console.warn('warn test'); console.error('error test');")

# 3. Wait for events to propagate
time.sleep(0.5)

# 4. Collect logs
logs = tab.get_console_logs()
print(f"\n  Captured {len(logs)} console entries:")
for entry in logs:
    print(f"    [{entry['level'].upper():7s}] {entry['text']!r}  ts={entry['timestamp']}  src={entry['source']!r}")

# 5. Assertions
results: list[tuple[str, bool]] = []

results.append(("len(logs) >= 3", len(logs) >= 3))
results.append(("any level=log", any(e["level"] == "log" for e in logs)))
results.append(("any level=warning", any(e["level"] == "warning" for e in logs)))
results.append(("any level=error", any(e["level"] == "error" for e in logs)))

# 6. Clear and verify
tab.clear_console_logs()
results.append(("get_console_logs() == [] after clear", tab.get_console_logs() == []))

# 7. Report
print()
all_pass = True
for label, ok in results:
    status = "PASS" if ok else "FAIL"
    if not ok:
        all_pass = False
    print(f"  [{status}] {label}")

print()
print(f"OVERALL: {'PASS' if all_pass else 'FAIL'}")

tab.close()
