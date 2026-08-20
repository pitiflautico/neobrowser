import asyncio
import os
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
HN_USER = "pitiflautico"
HN_PASS = "Nb!IqT4T6osqhI6e4ul"

TITLE = "Honest bot-detection benchmark: real Chrome MCP vs Playwright MCP"

BODY = """I built NeoBrowser, an MCP server that drives the real Google Chrome binary over CDP, and I was tired of seeing every browser-automation tool claim it "passes bot detection" without saying where it fails.

So I ran a reproducible head-to-head against Playwright MCP on live anti-bot pages and published the raw numbers.

The setup:
- NeoBrowser 0.1.7 (real Chrome via CDP)
- Playwright MCP via npx @playwright/mcp@latest --headless
- Same machine, same IP, no proxies
- Same harness and wall classifier
- N=2 per cell

Targets: bot.sannysoft.com, creepjs, nowsecure.nl (real Cloudflare), deviceandbrowserinfo.com.

The honest table:
- sannysoft: NeoBrowser 11/11, Playwright MCP 10/11 (fails UA: HeadlessChrome)
- nowsecure.nl: both blocked by Cloudflare
- latency: Playwright MCP ~1s, NeoBrowser ~4s

The uncomfortable truths are the useful ones: Cloudflare blocked both tools from a single residential IP, and Playwright is faster. NeoBrowser's edge is not bypassing strangers' walls; it's driving your already-logged-in browser for your own accounts and workflows.

Full methodology, raw JSON, and the harness: https://github.com/pitiflautico/neobrowser/blob/main/bench/study.md

I’d rather be called out for a bad measurement than quietly overclaim."""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:400]
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

            await call(session, "navigate", {"url": "https://news.ycombinator.com/login", "wait_s": 4})
            await call(session, "fill", {"selector": "input[name='acct']", "value": HN_USER})
            await call(session, "fill", {"selector": "input[type='password'][name='pw']", "value": HN_PASS})
            await call(session, "click", {"selector": "input[type='submit']"})
            await asyncio.sleep(3)

            await call(session, "navigate", {"url": "https://news.ycombinator.com/submit", "wait_s": 5})

            await call(session, "fill", {"selector": "input[name='title']", "value": TITLE})
            await call(session, "fill", {"selector": "textarea[name='text']", "value": BODY})

            r = await call(session, "js", {"code": """
                const form = document.querySelector('form');
                const submit = form ? form.querySelector('input[type="submit"], button[type="submit"]') : null;
                if (submit) { submit.scrollIntoView({block:'center'}); submit.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("submit:", status)
            await asyncio.sleep(6)

            # verify on user submissions
            await call(session, "navigate", {"url": "https://news.ycombinator.com/submitted?id=pitiflautico", "wait_s": 5})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "Honest bot-detection benchmark" in text:
                print("\n=== POST VERIFIED ===")
            else:
                print("\n=== POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
