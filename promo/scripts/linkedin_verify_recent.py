import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

async def call(session, name, args=None):
    args = args or {}
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:1200]
    return out

async def main():
    server_params = StdioServerParameters(
        command=os.path.expanduser("~/.local/bin/neobrowser"),
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "linkedin.com",
            "NEOBROWSER_LOG_LEVEL": "warn",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 10})
            await asyncio.sleep(5)
            text = await call(session, "read", {})
            markers = ["10.000 estrellas", "95/10.000", "deslogueaba de Gmail", "GMAIL_AT"]
            found = [m for m in markers if m in text]
            print("Found markers:", found)
            if found:
                print("\n=== POST APPEARS IN RECENT ACTIVITY ===")
            else:
                print("\n=== POST NOT FOUND ===")

if __name__ == "__main__":
    asyncio.run(main())
