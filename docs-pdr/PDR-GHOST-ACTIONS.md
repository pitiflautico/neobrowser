# PDR: Ghost Browser — 19 Actions for AI Agents

## Architecture

ghost.py es el motor. Mantiene Chrome vivo entre llamadas.
Cada action es un command en ghost.py que el MCP ghost.rs delega.

```
Claude Code → mcp__neoV2__ghost(action, params)
  → ghost.rs (Rust) → ghost.py (Python + undetected-chromedriver)
  → Chrome neomode (headless, CF bypass)
  → Result → JSON → Claude Code
```

## Actions (19)

| # | Action | Input | Output |
|---|--------|-------|--------|
| 1 | search | query, engine?, num? | [{title, url, snippet}] |
| 2 | navigate | url | {title, url, elements} |
| 3 | read | selector? | clean text (article mode) |
| 4 | find | text\|label\|role, index? | {found, tag, text, selector, clickable} |
| 5 | click | text\|selector\|index | {clicked, new_url?, new_title?} |
| 6 | type | selector\|text, value | {typed, field} |
| 7 | fill_form | {field: value, ...} | {filled: n, errors: []} |
| 8 | submit | selector? | {submitted, new_url} |
| 9 | screenshot | output? | file path |
| 10 | scroll | direction, amount? | {scrolled, new_content?} |
| 11 | extract_data | type(table\|list\|product) | structured JSON |
| 12 | login | url, email, password | {logged_in, title} |
| 13 | download | url\|selector, output? | file path |
| 14 | monitor | selector\|text, interval? | {changed, old, new} |
| 15 | api_intercept | url_pattern? | [{url, method, status, body}] |
| 16 | cookie_manage | action(list\|export\|import) | cookies |
| 17 | multi_tab | action(new\|switch\|list\|close) | tab info |
| 18 | wait_for | selector\|text\|network_idle | {found, elapsed_ms} |
| 19 | pipeline | [{action, params}, ...] | [{result}, ...] |
