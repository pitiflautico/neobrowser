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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "reddit.com",
            "NEOBROWSER_LOG_LEVEL": "warn",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            for url in POSTS:
                print(f"\n=== {url} ===")
                await call(session, "navigate", {"url": url, "wait_s": 8})
                await asyncio.sleep(4)
                r = await call(session, "js", {"code": """
                    const comments = Array.from(document.querySelectorAll('.entry .md, .comment .md, div.usertext-body'));
                    const authors = Array.from(document.querySelectorAll('.entry .author, .comment .author'));
                    return comments.map((c,i) => ({
                        author: (authors[i] && authors[i].textContent) || 'unknown',
                        text: c.textContent.trim().slice(0,300)
                    }));
                """})


if __name__ == "__main__":
    asyncio.run(main())
