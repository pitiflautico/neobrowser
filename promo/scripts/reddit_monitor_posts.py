import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
POSTS = [
    "https://old.reddit.com/r/mcp/comments/1vtpi7j/",
    "https://old.reddit.com/r/SideProject/comments/1vtpse8/",
]


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:1000]
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

            for url in POSTS:
                print(f"\n=== CHECKING {url} ===")
                await call(session, "navigate", {"url": url, "wait_s": 8})
                await asyncio.sleep(4)
                r = await call(session, "read", {})
                text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
                # look for comment count indicators
                print("\n--- COMMENTS EXCERPT ---")
                # print a chunk around comments
                print(text[:3000])


if __name__ == "__main__":
    asyncio.run(main())
