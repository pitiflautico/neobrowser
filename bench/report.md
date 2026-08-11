# NeoBrowser benchmark — first pass

Tool: `neobrowser` · tasks: 12 · single IP, single run (see honest limits in run.py).

| metric | value |
|---|---|
| task success rate | **12/12 = 100%** |
| bot-wall detection rate | 3/12 |
| avg latency / task | 3965 ms |
| crashes | 0 |
| crash-recovery (self-heal) | PASS |

## Per task

| task | category | result | latency | wall |
|---|---|---|---|---|
| nav-basic | navigation | ✓ | 3593ms |  |
| login | login | ✓ | 4765ms | login_required |
| upload | upload | ✓ | 3635ms |  |
| extract-table | extract | ✓ | 2969ms |  |
| spa-dynamic-load | spa | ✓ | 8267ms |  |
| long-nav-read | long-navigation | ✓ | 3301ms |  |
| json-api | navigation | ✓ | 3431ms |  |
| botwall-detection | bot-wall | ✓ | 3917ms | bot_wall |
| cloudflare-detection | cloudflare | ✓ | 3199ms | captcha |
| multitab | multi-tab | ✓ | 3340ms |  |
| recovery | crash-recovery | ✓ | 5578ms |  |
| persistence | persistent-session | ✓ | 1589ms |  |
