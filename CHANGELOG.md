# Changelog

All notable changes to NeoBrowser are documented here.

## [3.1.0] - 2026-03-31

### Added
- CLI: `--help`, `--version`, `doctor` commands
- Dedicated CDP tabs for ChatGPT and Grok (persist across browse operations)
- DOM-based response detection (replaces slow title polling)
- HTTP fallback in browse tool (avoids Chrome for simple pages)
- readyState polling in open tool (no more fixed sleeps)
- Radio button support in fill tool (label[for] matching + change events)
- Smart find with targeted selectors and specificity sorting
- YAML plugin system for reusable automation pipelines
- Benchmark suite (tools/v3/benchmark.py)
- Landing page (EN + ES) at pitiflautico.github.io/neobrowser
- npm package: npx neobrowser

### Changed
- Session sync now logs exactly what gets copied (cookies kept/excluded count)
- sanitize() text limit increased 2000→4000 chars (more content for LLMs)
- save() inline limit increased 500→4000 chars (LLM gets full context)
- extract:links output changed from JSON to compact text format (-54% tokens)
- GPT responses now return only the last assistant message, not full history
- Chrome launch uses polling instead of fixed sleep(2)
- Tab creation uses polling instead of fixed sleep(1)

### Fixed
- ChatGPT streaming hang caused by SSE TransformStream interceptor (removed)
- fill tool reported success for radios without actually selecting them
- find tool returned empty results for text that existed on page
- tool_type cleared contentEditable fields (now only clears INPUT/TEXTAREA)
- Chrome processes leaked on unexpected exit (improved signal handling)

### Performance
- Total benchmark: 91.5s → 41.7s (-54%)
- GPT round-trip: 64.3s → 7-33s (dedicated tab + DOM detection)
- open average: 3.7s → 1.0s (readyState polling)
- Cold start: 2.0s → 1.6s (launch polling)

## [3.0.0] - 2026-03-29

### Added
- NeoBrowser V3: complete rewrite as Python MCP server
- Ghost Chrome: headless Chrome per MCP process with isolated profiles
- Session sync: cookies, localStorage, IndexedDB from real Chrome
- 19 MCP tools: browse, search, open, read, find, click, type, fill, submit, scroll, wait, login, extract, screenshot, js, gpt, grok, plugin, status
- ChatGPT and Grok integration via dedicated browser tabs
- Smart element detection (__neoFind) with multi-signal scoring
- Anti-bot: real Chrome UA, disabled automation flags
- Cloudflare bypass via real Chrome TLS

## [0.3.1] - 2026-03-15

- Previous Rust-based engine (archived)
