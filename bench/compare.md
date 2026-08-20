# NeoBrowser vs Playwright MCP — neutral 2-way benchmark

Common layer drives both tools with identical abstract steps + JS. Single machine, single run.

`task_execution_success` = the steps ran; `destination_access_success` = the intended content was actually reached (a walled/blocked destination is exec-success but access-failure).

## Functional

| task | NeoBrowser exec / access / calls / ms | Playwright MCP exec / access / calls / ms |
|---|---|---|
| nav_read | ✓ / ✓ / 3 / 3705 | ✓ / ✓ / 3 / 1749 |
| login | ✓ / ✓ / 6 / 6335 | ✓ / ✓ / 6 / 6005 |
| dom_extract | ✓ / ✓ / 3 / 2941 | ✓ / ✓ / 3 / 1528 |
| spa_dynamic | ✓ / ✓ / 12 / 8194 | ✓ / ✓ / 8 / 7293 |
| screenshot | ✓ / ✓ / 3 / 2600 | ✓ / ✓ / 3 / 605 |
| multitab | ✓ / ✓ / 6 / 58750 | ✓ / ✓ / 6 / 1145 |
| upload | ✓ / ✓ / 5 / 6133 | ✓ / ✓ / 6 / 4911 |
| persistence | ✓ / ✓ / 6 / 7943 | ✓ / ✓ / 6 / 4194 |
| recovery | ✓ / ✓ / 4 / 6837 | ✓ / ✓ / 4 / 2150 |

**Summary**

| tool | exec success | access success | avg calls | avg ms | crashes | recovery |
|---|---|---|---|---|---|---|
| NeoBrowser | 9/9 | 9/9 | 5.3 | 11493 | 0 | PASS |
| Playwright MCP | 9/9 | 9/9 | 5.0 | 3287 | 0 | PASS |

## Adversarial (observational — no bypass claim)

| task | NeoBrowser wall / access | Playwright MCP wall / access |
|---|---|---|
| google_images | bot_wall / ✗ | bot_wall / ✗ |
| cloudflare_nowsecure | captcha / ✗ | captcha / ✗ |

_Adversarial rows are single-IP, single-run observations. No 'evades better' claim is made — that needs residential-proxy IP rotation + N repetitions + a large site sample._

## Honest reading of these numbers

- **Latency:** NeoBrowser is consistently slower on these tasks. Part of that is a *deliberate trade-off*: it forces compositor frames (`nudge_frame`) so deferred/virtualized content actually renders in headless Chrome, which Playwright MCP skips, and it can be tuned down where content is static. Do not read the average as a clean multiple, though — see the caveat below, and note that a single outlier moves it substantially.
- **upload:** both tools are now driven through their native path — NeoBrowser via CDP `setFileInputFiles`, Playwright via a Playwright-driven click that arms its file-chooser interception, answered by `browser_file_upload`. Playwright MCP also restricts file access to its workspace roots, so the server is started with its cwd at the fixture directory rather than by passing `--allow-unrestricted-file-access`: pointing a competitor at the right root is fair, switching off its security control is not. The earlier revision of this harness clicked via JS, which Playwright never observes, and so scored a harness bug as a product failure.
- **persistence:** measured by outcome — does a persistent cookie survive a browser restart — not by whether a tool with a particular name exists. Playwright MCP is given `--user-data-dir`, its native persistence mechanism; NeoBrowser uses its Ghost profile. **The previous claim here ("a genuine capability gap") was wrong**: Playwright persists sessions perfectly well this way, and the old harness only concluded otherwise because it demanded a `save_cookies`/`restore_cookies` tool.
- **shutdown method matters:** the restart uses SIGTERM against the *browser* process only, for both tools. Chrome flushes its cookie store to SQLite during an orderly shutdown orchestrated by that process; a blanket `pkill -f` also kills the network-service child that owns the store, so nothing is flushed and a fully persistent profile reads as non-persistent. An intermediate revision of this harness made exactly that mistake and scored NeoBrowser as failing persistence.
- **recovery:** both tools recover (each relaunches its browser on the next navigate); this is *not* a NeoBrowser-only strength.
- **latency is not a clean product signal here:** `multitab` and `spa_dynamic` depend on third-party endpoints (`httpbin.org`, `the-internet.herokuapp.com`) that rate-limit by IP. A `multitab` figure in the tens of seconds is that throttle meeting NeoBrowser's longer navigation wait, not a tab-handling cost — the same sequence run in isolation completes in ~6s. Treat single-run latencies as indicative only; a trustworthy latency comparison needs a hermetic fixture server, which this harness does not yet have.
- **walls:** both detect the same walls and both were blocked on a single IP. NeoBrowser's edge is *surfacing* the wall type to the agent as a first-class signal, not bypassing it.
