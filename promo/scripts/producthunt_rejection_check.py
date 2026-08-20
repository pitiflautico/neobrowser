import asyncio
import os
import re
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"


def extract_bid(text):
    m = re.search(r'"backend_node_id"\s*:\s*(\d+)', text)
    if m:
        return int(m.group(1))
    return None


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:1200]
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "producthunt.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 10})
            await asyncio.sleep(3)

            # accept cookies
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /accept all/i.test(x.textContent));
                if (b) { b.click(); return 'ACCEPTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            r = await call(session, "find", {"intent": "product url input"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
                await asyncio.sleep(1)
                await call(session, "type", {"text": "https://gentle-khapse-c58c79.netlify.app/", "human": True})
                await asyncio.sleep(2)

            r = await call(session, "find", {"intent": "get started button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            await asyncio.sleep(8)

            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- FULL PAGE TEXT ---")
            print(text)


if __name__ == "__main__":
    asyncio.run(main())
