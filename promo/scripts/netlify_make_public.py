import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
OVERVIEW_URL = "https://app.netlify.com/projects/gentle-khapse-c58c79/overview"

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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "netlify.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": OVERVIEW_URL, "wait_s": 10})
            await asyncio.sleep(4)

            # click Make public
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /make public/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_MAKE_PUBLIC'; }
                return 'BTN_NOT_FOUND';
            """})
            await asyncio.sleep(6)

            # handle confirmation modal if any
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /make public|confirm|yes|continue/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_CONFIRM: ' + b.textContent.trim(); }
                return 'NO_CONFIRM';
            """})
            await asyncio.sleep(6)

            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- FINAL TEXT ---")
            print(text[:2000])


if __name__ == "__main__":
    asyncio.run(main())
