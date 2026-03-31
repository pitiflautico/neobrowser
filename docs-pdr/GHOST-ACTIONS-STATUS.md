# Ghost Actions — Test Results

## Tested and Working

| Action | Test | Result |
|--------|------|--------|
| **search (DDG)** | `ghost.py search "rust headless" --engine ddg --num 3` | 3 results con title/url/snippet |
| **navigate** | `ghost.py nav "https://news.ycombinator.com"` | 810 elements, 226 links, text preview |
| **read** | `ghost.py read "https://example.com"` | 17 words clean text extraction |
| **open** | `ghost.py open "https://grok.com"` | 358 elements, text visible |
| **fill_form** | `ghost.py fill_form "factorial login" --fields email+password` | 2/2 fields filled, CF bypassed |
| **extract links** | `ghost.py extract "HN" --type links` | 226 links structured JSON |
| **screenshot** | `ghost.py screenshot <url>` | PNG saved |
| **chat (GPT)** | `mcp__ai-chat__gpt` | "PONG" + persistent conversation |
| **chat (Grok)** | `mcp__ai-chat__grok` | Response received |

## Issues

| Issue | Impact | Fix |
|-------|--------|-----|
| **Zombie processes** | Each ghost.py call leaves chromedriver, blocking next call | Need singleton driver or aggressive PID cleanup |
| **Google search** | 0 results (consent page or CAPTCHA?) | Use DDG as default, investigate Google separately |
| **Sequential calls** | 2nd call always fails until zombies killed | Critical for MCP tool use |

## Not Yet Tested

| Action | Why |
|--------|-----|
| click | Zombie issue prevents testing |
| type | Zombie issue |
| scroll | Zombie issue |
| submit | Zombie issue |
| login | Zombie issue |
| download | Zombie issue |
| monitor | Zombie issue |
| api_intercept | Zombie issue |
| cookie_manage | Zombie issue |
| multi_tab | Zombie issue |
| wait_for | Zombie issue |
| pipeline | Zombie issue |
| find | Works but needs URL |

## Priority Fix: Zombie Management

The core issue: `undetected-chromedriver` spawns Chrome and chromedriver
processes that survive after `driver.quit()`. Each new ghost.py call
cannot launch Chrome because the port is still in use.

Solutions:
1. **Singleton pattern**: One Chrome for all ghost.py calls (like ai-chat mcp-server.py)
2. **Aggressive PID tracking**: Save Chrome PID, kill it on next invocation
3. **Port cycling**: Use random ports to avoid conflicts
