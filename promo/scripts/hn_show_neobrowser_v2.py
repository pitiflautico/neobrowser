import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TITLE = "Show HN: NeoBrowser – MCP server that drives real Chrome with your logged-in sessions"
TEXT = """I got tired of every browser MCP I tried launching a fresh headless browser and immediately hitting login walls. So I built one that drives your actual Chrome instead.

It connects over CDP, optionally injects your real cookies (opt-in, excludes identity cookies so you don't get logged out), and keeps the renderer sandbox on. The fingerprint is genuine because it's just your real browser.

Single 6.4MB Rust binary, 67 tools, MIT. I also ran a benchmark vs Playwright MCP — Playwright is faster, we do sessions and uploads it can't, both get walled equally on adversarial pages. Details in bench/.

Repo: https://github.com/pitiflautico/neobrowser"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:800]
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

            await call(session, "navigate", {"url": "https://news.ycombinator.com/submit", "wait_s": 8})
            await asyncio.sleep(4)

            # Fill title
            r = await call(session, "js", {"code": f"""
                const title = document.querySelector('input[name="title"]');
                if (!title) return 'NO_TITLE';
                title.focus();
                title.value = {TITLE!r};
                title.dispatchEvent(new Event('input', {{bubbles:true}}));
                return 'TITLE_SET';
            """})
            await asyncio.sleep(1)

            # Fill url
            r = await call(session, "js", {"code": """
                const url = document.querySelector('input[name="url"]');
                if (!url) return 'NO_URL';
                url.focus();
                url.value = 'https://github.com/pitiflautico/neobrowser';
                url.dispatchEvent(new Event('input', {bubbles:true}));
                return 'URL_SET';
            """})
            await asyncio.sleep(1)

            # Fill text
            r = await call(session, "js", {"code": f"""
                const text = document.querySelector('textarea[name="text"]');
                if (!text) return 'NO_TEXT';
                text.focus();
                text.value = {TEXT!r};
                text.dispatchEvent(new Event('input', {{bubbles:true}}));
                return 'TEXT_SET';
            """})
            await asyncio.sleep(1)

            # Submit
            r = await call(session, "js", {"code": """
                const form = document.querySelector('form');
                if (!form) return 'NO_FORM';
                form.submit();
                return 'FORM_SUBMITTED';
            """})
            await asyncio.sleep(8)

            # Verify by checking if we're on the new item page
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "NeoBrowser" in text and ("github.com/pitiflautico/neobrowser" in text or "Show HN" in text):
                print("\n=== SHOW HN SUBMITTED ===")
            elif "story-toofast" in text or "toofast" in text:
                print("\n=== RATE LIMITED ===")
            else:
                print("\n=== UNKNOWN STATE ===")


if __name__ == "__main__":
    asyncio.run(main())
