import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TITLE = "How I stopped my browser agent from logging users out of their real Chrome"
BODY = """I built an MCP server that drives the user's real Chrome. The first version imported every cookie from the real profile into the automated browser. Google detected the cloned session and revoked the original browser's login.

The fix: aggressive filtering of identity cookies (Gmail's GMAIL_AT, OSID) and fingerprint cookies (AEC, SOCS, 1P_JAR). Now we only inject what's needed for the target domain, and we exclude session-identity cookies by default.

Lesson: in browser agents, "more real session" isn't always "more stealth". Platforms detect inconsistency between browsers.

Repo if you're curious: https://github.com/pitiflautico/neobrowser"""


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "reddit.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://old.reddit.com/r/mcp/submit?selftext=true", "wait_s": 8})
            await asyncio.sleep(4)

            # Fill title
            r = await call(session, "js", {"code": f"""
                const title = document.querySelector('textarea[name="title"]');
                if (!title) return 'NO_TITLE';
                title.focus();
                title.value = {TITLE!r};
                title.dispatchEvent(new Event('input', {{bubbles:true}}));
                return 'TITLE_SET';
            """})
            await asyncio.sleep(1)

            # Fill body
            r = await call(session, "js", {"code": f"""
                const body = document.querySelector('textarea[name="text"]');
                if (!body) return 'NO_BODY';
                body.focus();
                body.value = {BODY!r};
                body.dispatchEvent(new Event('input', {{bubbles:true}}));
                return 'BODY_SET';
            """})
            await asyncio.sleep(1)

            # Submit
            r = await call(session, "js", {"code": """
                const form = document.querySelector('form#newlink');
                if (!form) return 'NO_FORM';
                form.submit();
                return 'FORM_SUBMITTED';
            """})
            await asyncio.sleep(8)

            # Verify
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "session hygiene" in text or "GMAIL_AT" in text or "NeoBrowser" in text:
                print("\n=== REDDIT POST VERIFIED ===")
            elif "you are doing that too much" in text or "rate limit" in text.lower():
                print("\n=== RATE LIMITED ===")
            else:
                print("\n=== UNKNOWN STATE ===")


if __name__ == "__main__":
    asyncio.run(main())
