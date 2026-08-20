import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
SUBREDDIT = "mcp"
GIF_URL = "https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif"

TITLE = "[Showcase] NeoBrowser — MCP server that drives your real Chrome with your real sessions"
BODY = f"""Hey r/mcp,

We've been hitting a wall with agents and real websites: the moment a site needs a logged-in session, a fresh headless browser becomes useless.

NeoBrowser is an MCP server that drives *your* actual Chrome (or launches a real one) with your real profiles and sessions. It exposes the usual tools — navigate, click, type, screenshot, extract, search — but the browser behind them is genuinely yours, not a sterile puppet.

Key bits:
- Single static Rust binary (~6.4 MB), zero runtime dependencies.
- Real Chrome with real sessions (attach to your own or let it launch one).
- Genuine anti-detection: real WebGL, real permissions, real trust signals — no spoofing.
- Verified-action contract + audit log for destructive ops.
- Honest benchmark vs Playwright MCP published in the repo.

GIF (15s): {GIF_URL}
Repo: https://github.com/pitiflautico/neobrowser

We're at 89 GitHub stars. Happy to answer questions or take punches on the benchmark methodology."""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:400]
    print(f"< {out}")
    return r


async def main():
    server_params = StdioServerParameters(
        command=os.path.expanduser("~/.local/bin/neobrowser"),
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

            await call(session, "navigate", {"url": f"https://old.reddit.com/r/{SUBREDDIT}/submit", "wait_s": 8})
            await asyncio.sleep(3)

            # reject cookie banner if present
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, a'));
                const b = btns.find(x => /reject|decline|only necessary/i.test(x.textContent));
                if (b) { b.click(); return 'REJECTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            # switch to text post tab
            await call(session, "js", {"code": """
                const textTab = document.querySelector('a.text-button');
                if (textTab) { textTab.click(); return 'CLICKED_TEXT'; }
                return 'NO_TEXT_TAB';
            """})
            await asyncio.sleep(2)

            # fill title and body
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

            # click submit
            await call(session, "js", {"code": """
                const form = document.querySelector('form.submit.content');
                const submit = form ? form.querySelector('button[type="submit"].btn') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(6)

            # verify
            await call(session, "navigate", {"url": "https://old.reddit.com/user/Pitiflautico2/submitted", "wait_s": 6})
            await asyncio.sleep(4)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if TITLE[:40] in text:
                print("\n=== REDDIT POST VERIFIED ===")
            else:
                print("\n=== REDDIT POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
