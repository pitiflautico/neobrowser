import asyncio
import os
import re
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

URLS = [
    "https://pitiflautico.github.io/neobrowser/?ref=ph",
    "https://pitiflautico.github.io/neobrowser/?utm_source=producthunt",
    "https://github.com/pitiflautico/neobrowser#readme",
    "https://github.com/pitiflautico/neobrowser/blob/main/README.md",
]


def extract_bid(text):
    m = re.search(r'"backend_node_id"\s*:\s*(\d+)', text)
    if m:
        return int(m.group(1))
    return None


async def call(session, name, args=None):
    args = args or {}
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:800]
    print(f"< {out}")
    return r


async def test_url(session, url):
    print(f"\n=== TESTING {url} ===")
    await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 8})
    await asyncio.sleep(3)

    r = await call(session, "find", {"intent": "product url input"})
    bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
    if bid:
        await call(session, "click", {"backend_node_id": bid})
        await asyncio.sleep(1)
        await call(session, "type", {"text": url, "human": True})
        await asyncio.sleep(2)

    r = await call(session, "find", {"intent": "get started button"})
    bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
    if bid:
        await call(session, "click", {"backend_node_id": bid})
    await asyncio.sleep(8)

    r = await call(session, "read", {})
    text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
    if "can't hunt" in text.lower() or "invalid" in text.lower():
        print(f"REJECTED: {url}")
        return False
    print(f"MAYBE ACCEPTED: {url}")
    print(text[:1500])
    return True


async def main():
    server_params = StdioServerParameters(
        command=os.path.expanduser("~/.local/bin/neobrowser"),
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "producthunt.com",
            "NEOBROWSER_LOG_LEVEL": "warn",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            for url in URLS:
                ok = await test_url(session, url)
                if ok:
                    break


if __name__ == "__main__":
    asyncio.run(main())
