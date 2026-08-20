import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
DROP_URL = "https://app.netlify.com/drop/gentle-khapse-c58c79"

async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:600]
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "netlify.com,github.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": DROP_URL, "wait_s": 10})
            await asyncio.sleep(4)

            # read the page
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- PAGE TEXT ---")
            print(text[:2000])
            print("--- END ---\n")

            # find buttons
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, a, div[role="button"], input[type="submit"]'));
                return btns.map(b => ({
                    tag: b.tagName,
                    text: (b.textContent || b.value || '').trim().slice(0,80),
                    href: b.href || '',
                    classes: b.className.slice(0,80)
                })).filter(x => x.text || x.href);
            """})
            print("BUTTONS:", " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:2000])

            # screenshot for manual review
            await call(session, "screenshot", {"path": os.path.expanduser("~/.neobrowser/promo-home/downloads/netlify-claim-recon.png"), "full_page": True})


if __name__ == "__main__":
    asyncio.run(main())
