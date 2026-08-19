import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

async def call(session, name, args=None):
    args = args or {}
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:400]
    print(f"{name}: {out}")
    return r

async def main():
    server_params = StdioServerParameters(
        command="/Users/danielperezpinazo/.local/bin/neobrowser",
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": "/tmp/nbpromo",
            "NEOBROWSER_REAL_PROFILE": "Profile 24",
            "NEOBROWSER_LOG_LEVEL": "error",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            await call(session, "navigate", {"url": "https://old.reddit.com/user/Pitiflautico2/submitted", "wait_s": 4})
            r = await call(session, "read", {"selector": "body"})
            r2 = await session.call_tool("screenshot", {"format": "png"})
            for c in r2.content:
                if getattr(c, "type", None) == "image":
                    import base64
                    with open("/tmp/reddit_submitted.png", "wb") as f:
                        f.write(base64.b64decode(c.data))
                    print("saved /tmp/reddit_submitted.png")

if __name__ == "__main__":
    asyncio.run(main())
