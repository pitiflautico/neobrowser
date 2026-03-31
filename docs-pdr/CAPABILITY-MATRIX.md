# V1 → V2 Capability Matrix

| Feature | V1 | V2 | Gap | Priority | Test |
|---|---|---|---|---|---|
| Page load (SSR) | ✅ 10/10 sites | ✅ 3/10 sites | 7 sites | P0 | test_v2_sites.sh |
| React hydration | ✅ ChatGPT 3.1s | ❌ | Full | P0 | test_hydration |
| Vue hydration | ✅ (V1 tested) | ❌ | Full | P1 | - |
| Module pre-fetch | ✅ depth 2, 362 | ❌ | Full | P0 | - |
| Module stubbing | ✅ 82MB saved | ❌ | Full | P0 | - |
| V8 bytecode cache | ✅ 10x speedup | Stub only | Full | P0 | - |
| Promise.allSettled rewrite | ✅ source transform | Stub only | Partial | P0 | - |
| Cookie persistence | ✅ SQLite | ✅ SQLite | None | - | cookies_test |
| HTTP cache | ✅ disk | ✅ disk | None | - | cache_test |
| Click | ✅ | ✅ stale recovery | None | - | interact_tests |
| Type | ✅ | ✅ contenteditable | None | - | interact_tests |
| Forms/CSRF | ✅ | ✅ auto-detect | None | - | interact_tests |
| Scroll | ✅ | ✅ infinite | None | - | interact_tests |
| Select/Checkbox | Partial | ✅ | V2 better | - | interact_tests |
| Popups/Consent | ✅ | ✅ auto-dismiss | None | - | interact_tests |
| Back/Forward | ❌ | ✅ history stack | V2 better | - | engine_tests |
| WOM extraction | ✅ | ✅ v2 (ARIA roles) | V2 better | - | extract_tests |
| Page classification | ✅ | ✅ v2 (12 types) | V2 better | - | extract_tests |
| Delta engine | ✅ | ✅ v2 (fingerprint) | V2 better | - | extract_tests |
| Structured extraction | ✅ | ✅ (prices, pagination) | V2 better | - | extract_tests |
| MCP server | ✅ 35+ tools | ✅ 4 tools | V1 more tools | P1 | mcp_tests |
| Chrome fallback | ✅ | ✅ (minimal) | V1 more features | P2 | chrome_tests |
| Telemetry skip | ✅ 73 patterns | ✅ 73 patterns | None | - | classify_test |
| Auth redaction | ❌ | ✅ traces | V2 better | - | trace_tests |
| Security boundary | ❌ | ✅ prototype freeze | V2 better | - | - |
| Resource limits | ❌ | ✅ config | V2 better | - | - |
| Pipeline contract | ❌ | ✅ (pending) | V2 better | - | - |
| Observability | ❌ | ✅ FileTracer + phase tracing | V2 better | - | trace_tests |
| Doubleclick | ❌ | ❌ | Missing | P2 | - |
| Right-click | ❌ | ❌ | Missing | P2 | - |
| File upload | ❌ | ❌ | Missing | P2 | - |
