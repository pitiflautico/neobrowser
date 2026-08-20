import asyncio
import os
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
HN_USER = "pitiflautico"
HN_PASS = "Nb!IqT4T6osqhI6e4ul"
ITEM = "49362047"

COMMENT = """This is a neat angle. I've been working on the opposite side of the same problem: getting AI agents to use the web without every site treating them as a brand new, untrusted browser.

One thing that surprised me while building NeoBrowser (an MCP server that drives real Chrome) is how much of "being trusted" is just having the same profile, cookies, and localStorage that the user already earned. For web apps that's straightforward; for native mobile apps it's harder because there's no universal CDP equivalent.

Question for you: are you injecting accessibility events / UIAutomator-style actions, or do you have a way to reuse the app's existing session state so the agent doesn't have to log in every time?

Disclosure: I built NeoBrowser. Genuine question — the mobile-vs-web split is interesting."""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name}")
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "news.ycombinator.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://news.ycombinator.com/login", "wait_s": 4})
            await call(session, "fill", {"selector": "input[name='acct']", "value": HN_USER})
            await call(session, "fill", {"selector": "input[type='password'][name='pw']", "value": HN_PASS})
            await call(session, "click", {"selector": "input[type='submit']"})
            await asyncio.sleep(3)

            await call(session, "navigate", {"url": f"https://news.ycombinator.com/item?id={ITEM}", "wait_s": 5})

            # find the main reply textarea (usually the first/biggest one)
            await call(session, "fill", {"selector": "textarea[name='text']", "value": COMMENT})
            await asyncio.sleep(1)

            r = await call(session, "js", {"code": """
                const form = document.querySelector('form[action="comment"]') || document.querySelector('form');
                const submit = form ? form.querySelector('input[type="submit"], button[type="submit"]') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("submit:", status)
            await asyncio.sleep(6)

            # verify comment visible on item page
            await call(session, "navigate", {"url": f"https://news.ycombinator.com/item?id={ITEM}", "wait_s": 5})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "NeoBrowser" in text and "mobile-vs-web split" in text:
                print("\n=== COMMENT VERIFIED ===")
            else:
                print("\n=== COMMENT NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
