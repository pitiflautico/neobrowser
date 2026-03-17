# Changelog

## v0.4.0 — Intelligence Layer

### New: Frame Support (CDP-level)
- `list_frames` — list all frames including cross-origin iframes with scores
- `switch_frame` — switch to frame by index or URL/name pattern
- `auto_frame` — auto-detect which frame contains target text
- `main_frame` — switch back to top-level page
- OOP frames via `Target.attachToTarget` with `flatten=true`
- Same-process frames via `Page.createIsolatedWorld` + `contextId`
- All actions (click, type, eval, observe) work inside active frame

### New: Form Analysis
- `analyze_forms` — detects all forms including Vue/React virtual forms
  - Extracts fields with name, type, required, label, placeholder, value
  - Detects Vue `v-model`, `data-vv-name` (vee-validate), `v-bind:value`
  - 5 methods for required detection: HTML attribute, aria-required, vee-validate, asterisk labels, CSS class
  - 6 label detection strategies
  - Full dropdown options extraction
- `analyze_api` — scans JS bundles for API endpoints
  - 11 regex patterns (fetch, axios, XHR, /api/, /v1/, /graphql/, etc.)
  - Checks SSR globals: `__NUXT__`, `__NEXT_DATA__`, `__INITIAL_STATE__`
  - Inspects Vuex/Pinia stores
  - Scans up to 3 external bundles (chunk/app/main)

### New: CDP Network Capture
- `capture_mode: "cdp"` — survives navigation, captures cross-origin iframe requests
- `Network.enable` + listeners for `requestWillBeSent`/`responseReceived`
- `body` op — get response body by requestId via `Network.getResponseBody`
- `intercept` op — mock API responses via `Fetch.enable` + `Fetch.fulfillRequest`
- `clear_intercepts` — remove all mock rules
- Three modes: `js` (default), `cdp`, `both`

### New: Workflow Mapper (browser_learn)
- `start` — begin workflow mapping session
- `observe` — rich observation with Vue/React state, form models, validation rules
- `act` — perform action with network capture, log before/after state
- `save` — export reusable JSON playbook
- `replay` — replay saved workflow with state verification at each step

### Fixed
- `eval_in_active_frame` infinite recursion bug
- `floor_char_boundary` replaced with stable `truncate_at()` for non-nightly Rust

### Internal
- `CdpSession.clone_tx()` — raw WebSocket send channel for event callbacks
- `CdpSession.shared_id_counter()` — shareable ID counter for callbacks
- `CdpNetworkEntry` struct for CDP-captured network events
- `InterceptRule` struct for URL pattern matching

## v0.3.0 — Stealth & Identity

- Pipe CDP (`--remote-debugging-pipe`) — bypasses Cloudflare Turnstile
- Polymorphic identity — OS-matched UA, GPU, screen, canvas, audio
- Cookie banner auto-dismiss via `Network.setBlockedURLs` + MutationObserver
- `Input.insertText` for Vue/React compatibility
- 16 new act kinds: pdf, drag, upload, clipboard, mouse, highlight, get_info, screenshot_annotated, device, geolocation, offline, color_scheme
- npm package: `npx neobrowser mcp`
- Docker image
- GitHub Actions CI: cross-compile macOS ARM/x86 + Linux

## v0.2.0 — MCP Foundation

- 13 MCP tools over stdio JSON-RPC
- WOM (Web Object Model) — semantic DOM compression
- 4-strategy click fallback
- Session persistence via Chrome profiles
- Light mode (HTTP only) and Auto mode
- Pipeline runner with retry and assertions
- Multi-context browser pool
- Action tracing with timing and stats

## v0.1.0 — Initial Release

- Raw CDP over WebSocket
- Basic navigation, click, type, eval
- Chrome launch with headless option
