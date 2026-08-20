import asyncio
import os
import re
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
OVERVIEW_URL = "https://app.netlify.com/projects/gentle-khapse-c58c79/overview"

async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:1200]
    print(f"< {out}")
    return r


def extract_bid(text):
    m = re.search(r'"backend_node_id"\s*:\s*(\d+)', text)
    if m:
        return int(m.group(1))
    return None


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

            # real click on Make public
            r = await call(session, "find", {"intent": "make public button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            print(f"Make public backend_node_id: {bid}")
            if bid:
                await call(session, "click", {"backend_node_id": bid})
                await asyncio.sleep(4)

            # look for confirm/make public in modal
            r = await call(session, "find", {"intent": "confirm make public"})
            bid2 = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            print(f"Confirm backend_node_id: {bid2}")
            if bid2:
                await call(session, "click", {"backend_node_id": bid2})
                await asyncio.sleep(6)

            # final read
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- FINAL TEXT ---")
            print(text[:2500])


if __name__ == "__main__":
    asyncio.run(main())
