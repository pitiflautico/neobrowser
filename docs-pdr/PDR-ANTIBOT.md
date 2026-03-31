# PDR: Access Layer — Browser-Required vs Headless Routing

## Problem (correctly framed)
Three SEPARATE problems were being mixed:
1. **Access** — can we reach the site's HTML? (blocked by Cloudflare/anti-bot)
2. **Session** — do we have valid auth cookies? (blocked by TLS-fingerprint binding)
3. **Execution** — can we hydrate/interact with the page? (runtime correctness, T1-T5)

These are independent layers. Solving one doesn't solve the others. Previous PDR was trying to make rquest "look like Chrome" — that's a dead end. Cloudflare correlates dozens of signals; coherence tweaks are marginal.

## Architecture: Access Router

```
AI requests "navigate to X"
    ↓
Access Router: classify site
    ↓
┌─────────────────────────┐
│ Route A: Headless        │ ← Sites without active protection
│ rquest + V8/linkedom     │ ← 80%+ of the web
│ Fast, no Chrome needed   │
└─────────────────────────┘
┌─────────────────────────┐
│ Route B: Chrome-assisted │ ← Sites with Cloudflare/Turnstile/challenge
│ Chrome CDP for access    │ ← Chrome solves challenge, gets cookies
│ Then: rquest for follow- │ ← rquest uses fresh cookies (may work)
│ up OR Chrome for all     │ ← Chrome for everything if cookies expire
└─────────────────────────┘
┌─────────────────────────┐
│ Route C: Direct API      │ ← Sites with known APIs (ChatGPT, LinkedIn)
│ rquest to API endpoint   │ ← No HTML needed, structured data
│ Token harvest if needed  │
└─────────────────────────┘
```

## Site Classification

### Detect protection level on first request
```rust
enum SiteAccess {
    Open,               // 200 OK, real HTML
    CloudflareManagedChallenge,  // CF challenge page detected
    CloudflareTurnstile, // Turnstile widget in page
    CaptchaRequired,    // Other CAPTCHA
    AuthRequired,       // 401/403, needs login
    ApiAvailable,       // Known API endpoint
}
```

Detection:
- Response contains `cf_chl_opt` or `/cdn-cgi/challenge-platform` → CloudflareManagedChallenge
- Response contains `cf-turnstile` widget → CloudflareTurnstile
- Response contains `<noscript>Enable JavaScript` + minimal HTML → Challenge page
- Status 403 + small body → likely blocked
- Known domain → route override (ChatGPT → API, LinkedIn → API)

### Route decision
| Classification | Route | Fallback |
|----------------|-------|----------|
| Open | A (headless) | — |
| CloudflareManagedChallenge | B (Chrome solve) | C (API) if available |
| CloudflareTurnstile | B (Chrome solve) | fail with clear error |
| AuthRequired | B (Chrome login) | fail with "login required" |
| ApiAvailable | C (API direct) | A or B |

## Implementation

### Layer 1: Challenge Detection (neo-http)
In `crates/neo-http/src/classify.rs` or new `challenge.rs`:

```rust
pub fn detect_challenge(status: u16, body: &str, headers: &HashMap<String,String>) -> SiteAccess {
    // Cloudflare managed challenge
    if body.contains("cf_chl_opt") || body.contains("/cdn-cgi/challenge-platform") {
        return SiteAccess::CloudflareManagedChallenge;
    }
    // Cloudflare Turnstile
    if body.contains("cf-turnstile") || body.contains("challenges.cloudflare.com/turnstile") {
        return SiteAccess::CloudflareTurnstile;
    }
    // Generic challenge (small body, noscript message)
    if body.len() < 10_000 && body.contains("Enable JavaScript") && body.contains("<noscript>") {
        return SiteAccess::CloudflareManagedChallenge;
    }
    // Auth required
    if status == 401 || status == 403 {
        return SiteAccess::AuthRequired;
    }
    SiteAccess::Open
}
```

### Layer 2: Chrome-assisted access (neo-chrome)
The `neo-chrome` crate already exists. Extend it with:

```rust
impl ChromeSession {
    /// Navigate Chrome to URL, let it solve any challenges, return cookies + HTML.
    pub async fn solve_and_extract(&mut self, url: &str) -> Result<(String, Vec<Cookie>), Error> {
        self.navigate(url).await?;
        // Wait for challenge to complete (poll document.readyState or URL change)
        self.wait_for_ready(30_000).await?;
        let html = self.get_html().await?;
        let cookies = self.get_cookies().await?;
        Ok((html, cookies))
    }
}
```

### Layer 3: Routing in NeoSession (neo-engine)
In `navigate()`:
1. Make request with rquest
2. Check response: `detect_challenge(status, body, headers)`
3. If `Open` → proceed with normal pipeline
4. If challenge → try Chrome-assisted:
   a. Launch Chrome (or reuse existing)
   b. Navigate + solve challenge
   c. Extract HTML + cookies
   d. Inject cookies into SQLite store
   e. Process HTML through normal V2 pipeline
5. If AuthRequired → return error with "login required via Chrome"

### Layer 4: Known API routes (optional, per-site)
For ChatGPT: use the V1 ai-chat API pattern (POST /backend-api/conversation)
For LinkedIn: use the V1 neoapi pattern

These are site-specific and optional. The core system works without them.

## Metrics (per layer, not mixed)

### Access
- HTTP status code (200 vs challenge vs blocked)
- Challenge detected? Type?
- Challenge solved? How? (headless / Chrome / API)
- Time to access

### Session
- Cookies injected? Count?
- Auth cookies present? (session-token, cf_clearance)
- Cookie TTL remaining

### Execution
- Runtime errors count
- WOM nodes extracted
- Framework detected
- Interactive elements found
- Hydration status

## What NOT to do
- Don't try to make rquest "look like Chrome" — diminishing returns
- Don't assume cf_clearance TTL is stable — revalidate as needed
- Don't mix access failures with runtime failures in metrics
- Don't add "realistic delays" without measuring their actual impact
- Don't build a generic anti-bot bypass — build access routing

## Phases

### Phase 1: Challenge detection
- Detect CF challenge in navigate() response
- Log access classification
- Return clear error for challenged sites
- Gate: detection works on ChatGPT, no false positives on open sites

### Phase 2: Chrome-assisted solve
- Integrate neo-chrome for challenge solving
- Chrome navigates → solves → extracts cookies+HTML
- Gate: ChatGPT accessed via Chrome solve + processed by V2 pipeline

### Phase 3: Smart routing
- Automatic route selection based on detection
- Cookie reuse across requests (until expiry)
- Retry logic: if rquest fails → try Chrome → if Chrome fails → clear error
- Gate: transparent to caller — navigate() just works

## Gate
- Challenge detection: 0 false positives on top 8 open sites
- Chrome solve: ChatGPT HTML obtained via Chrome CDP
- V2 processes Chrome-obtained HTML correctly
- Access/session/execution metrics separated in traces
- All existing tests pass
