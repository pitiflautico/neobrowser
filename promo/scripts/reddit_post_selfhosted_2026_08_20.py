import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
SUBREDDIT = "selfhosted"

TITLE = "NeoBrowser — self-hosted MCP server that drives your real Chrome instead of a cloud headless browser"

BODY = """I've been running local LLMs and MCP servers at home, but every browser MCP I tried either spawns a fresh headless Chrome (instant bot detection) or calls a cloud browser service (not self-hosted, not my session).

So I built NeoBrowser: a single static Rust binary that drives the real Google Chrome on your own machine over CDP. It stays local, uses your own Chrome profile if you opt in, and never phones home to a browser-as-a-service.

Why it fits here:

- Fully local. One ~6.4 MB binary. No Docker, no Node, no cloud API.
- Your sessions stay yours. Cookie import is opt-in and decrypts via your OS keychain (macOS/Linux/Windows). Identity cookies for Google/LinkedIn/Microsoft are excluded so your real browser doesn't get kicked out.
- Genuine fingerprint. It passes bot.sannysoft using the real Chrome binary and real GPU WebGL, not spoofed signals.
- Bot-wall aware. It detects CAPTCHA/consent/rate-limit/login gates and tells the model instead of hammering the page.
- 67 tools: navigate, forms, upload/download, screenshot, multi-tab, search, playbooks.

I benchmarked it against Playwright MCP with a neutral harness. Playwright is faster; NeoBrowser does session persistence and uploads that Playwright MCP can't. Full methodology is public in the repo.

Repo: https://github.com/pitiflautico/neobrowser (MIT)

Curious if others here are using MCP servers locally and what your setup looks like."""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:500]
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

            # click the real submit button
            r = await call(session, "js", {"code": """
                const form = document.querySelector('form.submit.content');
                const submit = form ? form.querySelector('button[type="submit"].btn') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("submit status:", status)
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
