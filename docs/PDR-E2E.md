# PDR: End-to-End Browser Functionality

## Status Quo
- Browser shim working: form.submit() triggers real HTTP navigation (tested on DDG)
- LiveDom with typed bridge, real events, robust targeting
- 8 MCP tools, interactive REPL, session loop
- ~300 tests, clippy clean
- BUG: extract after re-navigation returns stale content (uses old DOM/runtime)

## Critical Bug: Extract After Navigation

When form.submit() triggers navigation via `process_pending_navigations()`:
1. JS shim calls op_navigation_request with URL
2. Rust drains queue, calls self.navigate(url)
3. navigate() creates NEW V8 runtime, executes JS, extracts WOM
4. BUT: the REPL's `extract text` then calls `extract_text()` which uses LiveDom on the NEW runtime

The problem: after `process_pending_navigations()` calls `self.navigate()`, the runtime is replaced. The extract should work on the NEW runtime. Need to verify the runtime reference is properly updated.

Actual issue may be simpler: the REPL `extract text` calls `engine.extract_text()` which in `browser_impl.rs` reads from the runtime — if navigate() replaced self.runtime with a new one, extract_text should work. Let me trace the actual bug.

## Tier Tasks

### T1: Fix extract-after-navigation
- Debug why extract returns stale content after re-navigation
- Ensure runtime, DOM, and WOM are all from the new page
- Test: DDG type → submit → extract text → contains search results

### T2: Form Benchmark (5 real sites)
Test the full cycle: navigate → fill form → submit → verify outcome.

| Site | Form | Fields | Expected outcome |
|------|------|--------|------------------|
| DuckDuckGo | Search | q=query | Results page with titles |
| Google | Search | q=query | Results page (may need consent dismiss) |
| HN | Login | acct+pw | "Bad login" or redirect |
| GitHub | Login | login+password | Error message or redirect |
| httpbin.org/forms/post | POST form | All fields | Echo of submitted data |

Metrics per site:
1. Fields filled correctly (verify via eval)
2. Submit triggered (navigation request captured)
3. New page loaded (URL changed)
4. Expected content on result page (observable assertion)

### T3: ChatGPT Pong (external benchmark)
- Navigate chatgpt.com (cookies auto-imported)
- Find textarea (may be contenteditable div, not input)
- Type message
- Click send button
- Wait for response (poll for new assistant message)
- Extract reply

This is EXTERNAL benchmark — depends on ChatGPT UI which changes frequently. Not a gate blocker but validates the full stack.

### T4: Multi-page session test
Navigate → interact → navigate to different site → interact → verify cookies persisted.
- Start at site A → extract
- Navigate to site B → fill form → submit
- Navigate back to site A → verify session

### T5: MCP integration test
Test all 8 MCP tools work in sequence via the MCP server:
1. browse(url) → WOM
2. interact(click, selector) → result
3. interact(type, selector, text) → ok
4. interact(submit, selector) → navigation
5. extract(text) → page content
6. wait(selector, timeout) → found
7. eval(js) → result
8. search(query) → results

## Gate
- DDG search works end-to-end (type → submit → results extracted)
- 3/5 form sites pass all 4 metrics
- Multi-page session maintains cookies
- All existing tests pass
- ChatGPT pong: external, not blocking
