import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
THREAD = "https://news.ycombinator.com/item?id=49405405"

COMMENT = """This is a neat approach. Using desktop-level vision to click real Chrome sidesteps a lot of the fingerprinting headaches that headless browsers hit.

We've been chasing the same problem from a different angle: instead of looking at the screen, we attach to Chrome's DevTools Protocol and drive the browser while keeping the renderer sandbox and the user's real sessions. The tradeoff is real:

- Desktop vision / OS automation: works with any app, no browser integration needed, but needs screen access and can be brittle with window focus / scaling.
- CDP inside real Chrome: precise, fast, sandboxed, but you have to be paranoid about cookie/session hygiene or you log the user out of everything.

The session hygiene part is where we got burned. Early on we imported every cookie from the user's real profile. Google/LinkedIn detected the cloned session and revoked the real browser's login. Now we do opt-in per domain and exclude identity + fingerprint cookies by default.

Anyway, cool project. The more people building browser agents that don't rely on sterile headless instances, the better for everyone."""


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "news.ycombinator.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": THREAD, "wait_s": 8})
            await asyncio.sleep(4)

            await call(session, "js", {"code": """
                const ta = document.querySelector('textarea[name="text"]');
                if (!ta) return 'NO_TEXTAREA';
                ta.focus();
                ta.click();
                ta.scrollIntoView({block:'center'});
                return 'TEXTAREA_READY';
            """})
            await asyncio.sleep(1)

            await call(session, "type", {"text": COMMENT, "human": True})
            await asyncio.sleep(2)

            await call(session, "js", {"code": """
                const form = document.querySelector('form');
                if (!form) return 'NO_FORM';
                form.submit();
                return 'FORM_SUBMITTED';
            """})
            await asyncio.sleep(8)

            await call(session, "navigate", {"url": THREAD, "wait_s": 8})
            await asyncio.sleep(4)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "session hygiene part is where we got burned" in text or "sterile headless instances" in text:
                print("\n=== HN COMMENT VERIFIED ===")
            else:
                print("\n=== HN COMMENT NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
