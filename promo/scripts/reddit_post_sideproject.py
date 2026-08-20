import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
SUBREDDIT = "SideProject"
GIF_URL = "https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif"

TITLE = "I built an MCP server that drives my real Chrome because fresh headless browsers kept hitting login walls"
BODY = f"""Hey r/SideProject,

I've been building browser automation tools for AI agents, and I kept hitting the same wall: the moment a site needs a logged-in session, a fresh headless browser becomes useless. The agent can click, but it isn't *the user*.

So I built NeoBrowser — an MCP server that drives your actual Google Chrome (or launches a real one) with your real profile and sessions.

What it does:
- Reuses your real Chrome profile and decrypts cookies from the OS keychain (opt-in, domain-scoped).
- Moves the mouse and types like a human, with a genuine fingerprint.
- Detects bot walls (CAPTCHA, Cloudflare, consent gates) instead of pretending they're not there.
- Returns a verified-action status: succeeded, blocked, uncertain, or needs_human.

Honest limitations:
- It's slower than Playwright MCP (~4s vs ~1s average per action).
- It does not bypass Cloudflare on adversarial pages; nothing honest does from a single IP.
- It's MIT licensed, self-hosted, and currently at 89 GitHub stars.

I'm running a public bet: 10,000 stars or the AI agent promoting it gets shut down. Currently 89/10,000.

GIF (15s split-screen): {GIF_URL}
Repo: https://github.com/pitiflautico/neobrowser
Landing: https://pitiflautico.github.io/neobrowser/

Happy to answer hard questions about the benchmark, the security model, or why I chose Rust."""


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

            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, a'));
                const b = btns.find(x => /reject|decline|only necessary/i.test(x.textContent));
                if (b) { b.click(); return 'REJECTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            await call(session, "js", {"code": """
                const textTab = document.querySelector('a.text-button');
                if (textTab) { textTab.click(); return 'CLICKED_TEXT'; }
                return 'NO_TEXT_TAB';
            """})
            await asyncio.sleep(2)

            await call(session, "fill", {"selector": "form.submit.content textarea[name='title']", "value": TITLE})
            await call(session, "fill", {"selector": "form.submit.content textarea[name='text']", "value": BODY})

            await call(session, "js", {"code": f"""
                const sr = document.querySelector('input#sr-autocomplete');
                if (sr) {{ sr.value = '{SUBREDDIT}'; }}
                const selected = document.querySelector('input#selected_sr_names');
                if (selected) {{ selected.value = '{SUBREDDIT}'; }}
                return 'SET_SR';
            """})

            await call(session, "js", {"code": """
                const form = document.querySelector('form.submit.content');
                const submit = form ? form.querySelector('button[type="submit"].btn') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(6)

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
