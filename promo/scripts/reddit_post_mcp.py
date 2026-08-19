import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = "/tmp/nbpromo"
SUBREDDIT = "selfhosted"
TITLE = "NeoBrowser — self-hosted MCP server that drives your own Chrome for AI agents"
BODY = """If you're running local LLMs with tool use, you've probably noticed that "browser" tools either need cloud APIs or launch headless Chromium that fails on anything with a login wall.

NeoBrowser is a single Rust binary that acts as an MCP server and drives *your* Chrome. Attach to an existing profile (your sessions, cookies, extensions) or let it launch a fresh real Chrome. Everything stays local unless you explicitly navigate somewhere.

Repo: github.com/pitiflautico/neobrowser

Caveats are in the README: it's not faster than Playwright for pure scraping, and it won't bypass Cloudflare on sites that hate automation — nothing honest does. Questions welcome."""

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
            "NEOBROWSER_REAL_PROFILE": "Profile 24",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": f"https://old.reddit.com/r/{SUBREDDIT}/submit", "wait_s": 5})
            await call(session, "fill", {"selector": "input[name='title']", "value": TITLE})
            await call(session, "fill", {"selector": "textarea[name='text']", "value": BODY})
            await call(session, "find_and_click", {"text": "submit"})
            await asyncio.sleep(3)
            await call(session, "navigate", {"url": f"https://old.reddit.com/user/Pitiflautico2/submitted", "wait_s": 4})
            await call(session, "screenshot", {"format": "png"})

if __name__ == "__main__":
    asyncio.run(main())
