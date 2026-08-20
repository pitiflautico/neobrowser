import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
CLAIM_URL = "https://app.netlify.com/drop/gentle-khapse-c58c79/claim"

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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "netlify.com,github.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": CLAIM_URL, "wait_s": 12})
            await asyncio.sleep(4)

            # click Sign up with GitHub
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /Sign up with GitHub/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_GITHUB'; }
                return 'GITHUB_BTN_NOT_FOUND';
            """})
            await asyncio.sleep(10)

            # may redirect to github; wait and read current page
            for i in range(6):
                await asyncio.sleep(5)
                r = await call(session, "read", {})
                text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
                print(f"\n--- STEP {i} URL ---")
                r_url = await call(session, "js", {"code": "return location.href;"})
                print("URL:", " ".join(c.text if hasattr(c, "text") else str(c) for c in r_url.content))
                print("TEXT:", text[:1500])
                if "Authorize Netlify" in text or "Authorize" in text:
                    # click authorize
                    await call(session, "js", {"code": """
                        const btns = Array.from(document.querySelectorAll('button'));
                        const b = btns.find(x => /authorize/i.test(x.textContent));
                        if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_AUTHORIZE'; }
                        return 'AUTH_BTN_NOT_FOUND';
                    """})
                    await asyncio.sleep(10)
                if "Your project is live" in text and "claimed" in text.lower():
                    print("=== CLAIMED ===")
                    break
                if "Set up your site" in text or "site settings" in text.lower():
                    print("=== SETUP PAGE ===")
                    break

            # final state
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n=== FINAL TEXT ===")
            print(text[:3000])


if __name__ == "__main__":
    asyncio.run(main())
