import asyncio
import os
import random
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = "/tmp/nbpromo"
PROFILE = "Profile 24"

SUBREDDITS = ["selfhosted", "mcp"]
GIF_URL = "https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif"

TITLES = [
    "NeoBrowser — self-hosted MCP server that drives your own Chrome for AI agents",
    "[Showcase] NeoBrowser — MCP server that drives your real Chrome instead of a blank headless browser",
]

BODIES_SELFHOSTED = [
    f"""If you're running local LLMs with tool use, you've probably noticed that "browser" tools either need cloud APIs or launch headless Chromium that fails on anything with a login wall.

NeoBrowser is a single Rust binary that acts as an MCP server and drives *your* Chrome. Attach to an existing profile (your sessions, cookies, extensions) or let it launch a fresh real Chrome. Everything stays local unless you explicitly navigate somewhere.

GIF of the difference (15s): {GIF_URL}

Repo: github.com/pitiflautico/neobrowser

Caveats are in the README: it's not faster than Playwright for pure scraping, and it won't bypass Cloudflare on sites that hate automation — nothing honest does. Questions welcome.""",
]

BODIES_MCP = [
    f"""Hey r/mcp,

We've been hitting a wall with agents and real websites: the moment a site needs a logged-in session, a fresh headless browser becomes useless.

NeoBrowser is an MCP server that drives *your* actual Chrome (or launches a real one) with your real profiles and sessions. It exposes the usual tools — navigate, click, type, screenshot, extract, search — but the browser behind them is genuinely yours, not a sterile puppet.

Key bits:
- Single static Rust binary, zero runtime dependencies.
- Real Chrome with real sessions (attach to your own or let it launch one).
- Genuine anti-detection: real WebGL, real permissions, real trust signals — no spoofing.
- Verified-action contract + audit log for destructive ops.
- Honest benchmark vs Playwright MCP published in the repo.

GIF: {GIF_URL}
Repo: github.com/pitiflautico/neobrowser

We're at 88 GitHub stars. Happy to answer questions or take punches on the benchmark methodology.""",
]

SUBREDDIT = random.choice(SUBREDDITS)
if SUBREDDIT == "selfhosted":
    TITLE = TITLES[0]
    BODY = BODIES_SELFHOSTED[0]
else:
    TITLE = TITLES[1]
    BODY = BODIES_MCP[0]


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:400]
    print(f"< {out}")
    return r


async def main():
    server_params = StdioServerParameters(
        command="/Users/danielperezpinazo/.local/bin/neobrowser",
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "reddit.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": f"https://old.reddit.com/r/{SUBREDDIT}/submit", "wait_s": 5})

            # reject cookie banner if present
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, a'));
                const b = btns.find(x => /reject|decline|only necessary/i.test(x.textContent));
                if (b) { b.click(); return 'REJECTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(1)

            # switch to text post tab
            await call(session, "js", {"code": """
                const textTab = document.querySelector('a.text-button');
                if (textTab) { textTab.click(); return 'CLICKED_TEXT'; }
                return 'NO_TEXT_TAB';
            """})
            await asyncio.sleep(2)

            # fill title (textarea with name="title") and body
            await call(session, "fill", {"selector": "form.submit.content textarea[name='title']", "value": TITLE})
            await call(session, "fill", {"selector": "form.submit.content textarea[name='text']", "value": BODY})

            # ensure subreddit is selected
            await call(session, "js", {"code": f"""
                const sr = document.querySelector('input#sr-autocomplete');
                if (sr) {{ sr.value = '{SUBREDDIT}'; }}
                const selected = document.querySelector('input#selected_sr_names');
                if (selected) {{ selected.value = '{SUBREDDIT}'; }}
                return 'SET_SR';
            """})

            # click the real submit button
            await call(session, "js", {"code": """
                const form = document.querySelector('form.submit.content');
                const submit = form ? form.querySelector('button[type="submit"].btn') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(5)

            # verify
            await call(session, "navigate", {"url": "https://old.reddit.com/user/Pitiflautico2/submitted", "wait_s": 4})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if TITLE[:40] in text:
                print(f"\n=== POST VERIFIED ON /user/Pitiflautico2/submitted ===")
            else:
                print(f"\n=== POST NOT FOUND; check manually ===")


if __name__ == "__main__":
    asyncio.run(main())
