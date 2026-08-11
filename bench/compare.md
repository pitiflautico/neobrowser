# NeoBrowser vs Playwright MCP — neutral 2-way benchmark

Common layer drives both tools with identical abstract steps + JS. Single machine, single run.

`task_execution_success` = the steps ran; `destination_access_success` = the intended content was actually reached (a walled/blocked destination is exec-success but access-failure).

## Functional

| task | NeoBrowser exec / access / calls / ms | Playwright MCP exec / access / calls / ms |
|---|---|---|
| nav_read | ✓ / ✓ / 3 / 3156 | ✓ / ✓ / 3 / 2154 |
| login | ✓ / ✓ / 6 / 5694 | ✓ / ✓ / 6 / 5503 |
| dom_extract | ✓ / ✓ / 3 / 2972 | ✓ / ✓ / 3 / 1398 |
| spa_dynamic | ✓ / ✓ / 12 / 8210 | ✓ / ✓ / 8 / 7210 |
| screenshot | ✓ / ✓ / 3 / 2640 | ✓ / ✓ / 3 / 571 |
| multitab | ✓ / ✓ / 6 / 5670 | ✓ / ✓ / 6 / 1048 |
| upload | ✓ / ✓ / 5 / 5417 | ✗ / ✗ / 6 / 3074 |
| persistence | ✓ / ✓ / 5 / 2578 | ✗ / ✗ / 3 / 23 |
| recovery | ✓ / ✓ / 4 / 6499 | ✓ / ✓ / 4 / 2389 |

**Summary**

| tool | exec success | access success | avg calls | avg ms | crashes | recovery |
|---|---|---|---|---|---|---|
| NeoBrowser | 9/9 | 9/9 | 5.2 | 4760 | 0 | PASS |
| Playwright MCP | 7/9 | 7/9 | 4.7 | 2597 | 0 | PASS |

## Adversarial (observational — no bypass claim)

| task | NeoBrowser wall / access | Playwright MCP wall / access |
|---|---|---|
| google_images | bot_wall / ✗ | bot_wall / ✗ |
| cloudflare_nowsecure | captcha / ✗ | captcha / ✗ |

_Adversarial rows are single-IP, single-run observations. No 'evades better' claim is made — that needs residential-proxy IP rotation + N repetitions + a large site sample._

## Honest reading of these numbers

- **Latency:** NeoBrowser is ~2× slower on several tasks. That is a *deliberate trade-off*, not a defect: it forces compositor frames (`nudge_frame`) so deferred/virtualized content actually renders in headless Chrome — Playwright MCP skips that. It can be tuned down where content is static.
- **upload:** Playwright's failure here is partly this harness's neutral JS-click mapping, which does not arm Playwright's native file-chooser (it expects a Playwright-driven click on the input). NeoBrowser uploads via CDP `setFileInputFiles`, which is chooser-independent. Read as: NeoBrowser's upload path is simpler, *not* that Playwright can't upload.
- **persistence:** a genuine capability gap — Playwright MCP exposes no cookie save/restore tool.
- **recovery:** both tools recover (each relaunches its browser on the next navigate); this is *not* a NeoBrowser-only strength.
- **walls:** both detect the same walls and both were blocked on a single IP. NeoBrowser's edge is *surfacing* the wall type to the agent as a first-class signal, not bypassing it.
