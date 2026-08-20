import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
ITEM = "47734871"  # "I benchmarked MCP vs. CLI for browser automation. MCP wins by 25x"
COMMENT = """Interesting benchmark. One dimension I'd love to see added for browser MCPs is what "passes" actually means: sannysoft score, successful login on a real site, or completing a task without triggering a challenge. 

Also, fresh-profile vs. session continuity changes the numbers a lot for anything behind a login wall. A tool that reuses the user's existing browser state can look slower per command but skip the expensive authentication/reputation-building phase entirely.

Would be great to have a shared task matrix so different implementations can be compared on the same page."""


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "news.ycombinator.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": f"https://news.ycombinator.com/item?id={ITEM}", "wait_s": 8})
            await asyncio.sleep(3)

            # find reply box (first one under post)
            r = await call(session, "js", {"code": """
                const ta = document.querySelector('textarea[name="text"]');
                if (ta) { ta.focus(); ta.click(); return 'FOCUSED'; }
                return 'NO_TEXTAREA';
            """})
            await asyncio.sleep(1)

            await call(session, "type", {"text": COMMENT, "human": True})
            await asyncio.sleep(2)

            r = await call(session, "js", {"code": """
                const form = document.querySelector('form[action="/comment"]');
                const btn = form ? form.querySelector('input[type="submit"]') : null;
                if (btn) { btn.click(); return 'CLICKED_SUBMIT'; }
                return 'SUBMIT_NOT_FOUND';
            """})
            await asyncio.sleep(6)

            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "comment-toofast" in text.lower():
                print("\n=== HN RATE-LIMITED ===")
            elif "Please try again" in text or "You're posting too fast" in text:
                print("\n=== HN RATE-LIMITED ===")
            else:
                print("\n=== HN COMMENT ATTEMPT COMPLETE ===")
                print(text[:1500])


if __name__ == "__main__":
    asyncio.run(main())
