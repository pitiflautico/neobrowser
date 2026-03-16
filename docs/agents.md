# Building Agents on NeoBrowser

## Agent Architecture

```
Goal → Planner → Pipeline → Executor → Observer → Result
                                 ↑          |
                                 └── mutate ←┘
```

Claude (or any LLM) acts as planner + observer. NeoBrowser provides the executor layer.

## Pattern: LLM-Orchestrated Agent

The simplest pattern — Claude generates pipelines, NeoBrowser executes them.

```
1. Claude receives goal ("find IDOR on app.com")
2. Claude generates pipeline JSON
3. browser_pipeline executes it
4. Claude reads trace + results
5. Claude decides: done / retry / new pipeline
```

No custom agent code needed. Claude IS the agent.

## Pattern: Autonomous Pipeline

For recurring tasks, pre-define pipelines and run them without LLM in the loop.

```json
{
  "name": "check_login",
  "steps": [
    {"action": "goto", "target": "https://app.com"},
    {"action": "click", "target": "Login"},
    {"action": "type", "target": "email", "value": "{{email}}"},
    {"action": "type", "target": "password", "value": "{{password}}"},
    {"action": "click", "target": "Sign in", "assert_text": "Dashboard", "max_retries": 3},
    {"action": "extract", "value": "document.title", "store_as": "page_title"}
  ],
  "variables": {"email": "test@app.com", "password": "secret"}
}
```

## Agent 1: Recon Agent (Bounty Hunting)

### Capabilities needed
- Crawl and discover endpoints
- Capture and analyze network traffic (HAR)
- Test auth boundaries (access without token, with different roles)
- Fuzz parameters
- Validate findings

### Workflow
```
1. browser_open target site
2. browser_network start
3. Navigate key flows (login, dashboard, profile, admin)
4. browser_network read → analyze HAR for API endpoints
5. browser_api to replay interesting calls without auth
6. browser_api to replay with modified params (IDOR, injection)
7. browser_trace stats → document findings
8. browser_state export → save session for later
```

### Example: IDOR check
```json
{
  "name": "idor_check",
  "steps": [
    {"action": "goto", "target": "https://app.com/profile/123"},
    {"action": "extract", "value": "document.body.innerText.substring(0,500)", "store_as": "own_profile"},
    {"action": "goto", "target": "https://app.com/profile/124"},
    {"action": "extract", "value": "document.body.innerText.substring(0,500)", "store_as": "other_profile"},
    {"action": "eval", "value": "'IDOR: ' + (document.body.innerText.includes('404') ? 'NO' : 'POSSIBLE')", "store_as": "result"}
  ]
}
```

## Agent 2: Extract Agent (Scraping)

### Capabilities needed
- Detect list pages (repeated DOM patterns)
- Detect pagination
- Infer data schema (title, price, link, image)
- Generate stable extractors
- Export structured data

### Workflow
```
1. browser_open target page
2. browser_act eval → detect repeated elements (CSS pattern)
3. browser_act eval → extract schema from first item
4. browser_pipeline → paginate and extract all items
5. Save to JSON/CSV
```

### Example: Product scraping
```json
{
  "name": "scrape_products",
  "steps": [
    {"action": "goto", "target": "https://shop.com/products"},
    {"action": "extract", "value": "JSON.stringify([...document.querySelectorAll('.product-card')].map(el => ({title: el.querySelector('h3')?.textContent?.trim(), price: el.querySelector('.price')?.textContent?.trim(), link: el.querySelector('a')?.href})))", "store_as": "products"},
    {"action": "click", "target": "Next", "on_fail": "skip"},
    {"action": "extract", "value": "JSON.stringify([...document.querySelectorAll('.product-card')].map(el => ({title: el.querySelector('h3')?.textContent?.trim(), price: el.querySelector('.price')?.textContent?.trim(), link: el.querySelector('a')?.href})))", "store_as": "products_p2"}
  ]
}
```

## Agent 3: Monitor Agent (Web App Monitoring)

### Capabilities needed
- Run pipeline on schedule
- Capture state snapshots
- Compare against baseline
- Alert on changes

### Workflow
```
1. browser_pipeline → login + navigate to target page
2. browser_state export → snapshot
3. Compare with previous snapshot (diff cookies, localStorage, DOM)
4. browser_network read → check for errors, slow requests
5. browser_trace stats → check success rates
6. Alert if: login failed, content changed, new errors, degraded performance
```

### Example: Uptime check
```json
{
  "name": "health_check",
  "steps": [
    {"action": "goto", "target": "https://app.com", "assert_text": "Dashboard", "timeout_ms": 10000},
    {"action": "eval", "value": "document.querySelectorAll('.error, .alert-danger').length.toString()", "store_as": "error_count"},
    {"action": "eval", "value": "performance.timing.loadEventEnd - performance.timing.navigationStart + 'ms'", "store_as": "load_time"}
  ]
}
```

## Tool Composition Patterns

### Login → API scraping (fastest)
```
browser_open login page
browser_act fill_form + click submit
browser_wait text_present="Dashboard"
browser_api url="/api/data" extract="json"  ← 10x faster than navigation
```

### Network-first recon
```
browser_network start
browser_open target
(navigate key flows)
browser_network har → analyze offline
browser_api replay interesting endpoints with modified params
```

### Trace-driven debugging
```
browser_trace start
(run complex automation)
browser_trace stats → which actions fail most?
browser_trace read → see exact failure points
```

### State persistence across sessions
```
browser_state export file="/tmp/session.json"
(later, new session)
browser_open about:blank
browser_state import file="/tmp/session.json"
browser_open target  ← already logged in
```

## Tips

- Use `browser_api` after login for API-level speed — don't navigate for data
- Use `browser_trace start` early — zero overhead, invaluable for debugging
- Use `browser_network start` before navigating to capture all initial requests
- Pipeline `on_fail: "skip"` for optional steps (cookie banners, popups)
- Pipeline `assert_text` as postconditions — catch failures early
- `browser_state health` before complex flows — detect stale sessions
- `click_reliable` (used by pipeline) tries 4 strategies — rarely fails
