# PDR: Virtual Action Layer — Interact Without Framework Hydration

## Problem
React/Vue/Svelte don't hydrate in our runtime. Event handlers never attach. DOM interaction (click, type) changes the DOM but doesn't trigger app logic.

BUT: SSR content is complete. We can READ everything. We can SET values. We just can't TRIGGER the app's fetch/submit/navigation via event handlers.

## Insight
A browser doesn't need React to submit a form. The browser collects form data and sends an HTTP request. We should do the same — bypass the framework layer entirely.

## Two Paths of Interaction

### Path A: DOM-level (works for native HTML)
- `<form action="/search">` → collect inputs → HTTP GET/POST → works ✅
- `<a href="/page">` → navigate → works ✅
- `<input type="checkbox">` → toggle checked → works ✅
- Works TODAY for: DuckDuckGo search, httpbin forms, HN login, static sites

### Path B: HTTP-level (works for SPAs without hydration)
For forms/actions that DON'T have HTML action= attributes and depend on JS handlers:
1. Read the page structure (SSR) — what fields exist, what buttons exist
2. Collect form data from DOM (values, checked, selected)
3. Determine the target endpoint:
   - From `form.action` if exists
   - From data attributes (`data-action`, `data-url`, etc.)
   - From page patterns (known APIs)
   - From network observation (if we can detect what fetch() calls the page makes)
4. Construct and send the HTTP request via rquest
5. Process the response (new HTML → re-parse, JSON → present to AI)

## What This Enables

### For ChatGPT
ChatGPT's send button triggers: `fetch('/backend-api/conversation', { method: 'POST', body: JSON, headers: auth })`

We can do this directly:
1. Read textarea value
2. Get auth token from cookies/page
3. POST to `/backend-api/conversation` via rquest
4. Parse streaming response
5. Return assistant message to AI

No React handlers needed. Same result.

### For Any SPA Form
1. AI reads the form via WOM (fields, types, required, etc.)
2. AI fills the fields via LiveDom (set values)
3. AI calls `submit_form` which:
   a. Tries Path A first (form.action → HTTP request)
   b. If no action: collects data, tries common API patterns
   c. If that fails: returns the filled form state to AI for manual API call

### For Login Forms
1. Read form structure (email/password fields, submit button)
2. Fill fields
3. Submit:
   - Path A: form has `action="/api/login"` → POST with credentials
   - Path B: no action → try POST to current URL with form data
   - Most login forms work with Path A because they're server-rendered

## Implementation

### V1: Smart Form Submit (enhance existing)

The existing `executeFormSubmit` in LiveDom already collects form data and POSTs. Enhance it:

```javascript
function smartSubmit(form, submitter) {
    // 1. Collect all form data
    const data = collectFormData(form, submitter);

    // 2. Determine action URL
    let action = (submitter?.formAction || form.action || '').trim();
    if (!action || action === 'javascript:void(0)' || action === '#') {
        // No real action — try current URL
        action = location.href;
    }

    // 3. Determine method
    const method = (submitter?.formMethod || form.method || 'GET').toUpperCase();

    // 4. Determine enctype
    const enctype = submitter?.formEnctype || form.enctype || 'application/x-www-form-urlencoded';

    // 5. Submit via navigation request (Rust handles the HTTP)
    _shimOps.op_navigation_request(JSON.stringify({
        url: action,
        method: method,
        form_data: data,
        enctype: enctype,
        type: 'form_submit'
    }));

    return { action, method, fields: Object.keys(data).length };
}
```

### V2: Direct API Call Tool

New MCP tool: `api_call` — for when the AI knows the endpoint:

```json
{
    "name": "api_call",
    "params": {
        "url": "/backend-api/conversation",
        "method": "POST",
        "headers": {"Content-Type": "application/json"},
        "body": "{\"message\": \"hello\"}",
        "use_page_cookies": true
    }
}
```

This uses the page's cookies + rquest TLS fingerprint to call any API. The AI decides the endpoint.

Implementation: in `crates/neo-mcp/src/tools/`, new `api_call.rs`:
- Uses the session's cookie store
- Uses rquest with Chrome136 fingerprint
- Returns response body (JSON, HTML, or text)
- AI can read and act on the response

### V3: API Discovery (future)

Automatically discover API endpoints by:
1. Scanning inline scripts for fetch/XHR calls
2. Checking `data-*` attributes on forms/buttons
3. Looking at `<link rel="preconnect">` for API domains
4. Pattern matching: `/api/*`, `/graphql`, `/_next/data/*`

For now: AI discovers endpoints from page content + knowledge.

## ChatGPT Pong via Virtual Action Layer

```
1. navigate("https://chatgpt.com")
   → SSR loads, 323 nodes, textarea visible

2. type(textarea, "Ping from NeoRender V2")
   → DOM value set

3. api_call({
     url: "https://chatgpt.com/backend-api/conversation",
     method: "POST",
     headers: {
       "Content-Type": "application/json",
       "Authorization": "Bearer {token_from_page}"
     },
     body: {
       "action": "next",
       "messages": [{
         "role": "user",
         "content": {"content_type": "text", "parts": ["Ping from NeoRender V2"]}
       }],
       "model": "gpt-4o"
     },
     use_page_cookies: true
   })
   → Response: streaming JSON with assistant message

4. extract(response)
   → "Pong"
```

No React hydration needed. Same result as a real browser.

## For Generic Sites

| Site type | Path | How |
|-----------|------|-----|
| Static HTML form | A | form.submit() → HTTP → works |
| Login form (server-rendered) | A | fill + submit → POST → redirect or error |
| Search form | A | fill + submit → GET with params → results page |
| SPA with known API | B | AI calls api_call directly |
| SPA with unknown API | B | AI reads page, discovers endpoint, calls api_call |
| SPA with complex state | — | Needs hydration (future work) |

## Implementation Order

### Phase 1: api_call MCP tool
- New tool that sends HTTP request with page cookies
- Returns response body
- AI decides endpoint/method/body

### Phase 2: Smart form submit enhancement
- Try harder to find action URL
- POST to current URL as fallback
- Support JSON content-type (for API forms)

### Phase 3: Token/auth extraction helpers
- `extract_token(pattern)` — find auth tokens in page HTML/cookies/scripts
- Common patterns: Bearer tokens, CSRF tokens, session IDs
- ChatGPT: `__Secure-next-auth.session-token` → Bearer token via /api/auth/session

## Gate
- api_call tool works with page cookies
- ChatGPT: POST to /backend-api/conversation returns response
- Login form on HN: fill + submit → response (Path A)
- Search on DDG: fill + submit → results (Path A, already works)
