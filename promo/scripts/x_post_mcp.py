import asyncio
import os
import random
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = "/tmp/nbpromo"
PROFILE = "Profile 24"
GIF = "/private/tmp/nbpromo/downloads/neobrowser-vs-headless.gif"

HOOKS = [
    "Most AI agents fail the moment a site asks for a login.",
    "Headless browsers leave fingerprints. Real Chrome doesn't.",
    "Watching an agent fail a captcha is the new watching paint dry.",
    "Your AI shouldn't need a fake browser to use the real web.",
]

BODIES = [
    "NeoBrowser drives *your* Chrome, with *your* real sessions. One binary. Zero setup.\n\nIf you've seen an agent bounce off a login wall, you get it.",
    "No spoofed UA. No puppeteer fingerprints. Just your actual browser, driven by an MCP server.\n\nAgents finally get to use the web like humans do.",
    "Real sessions mean real access. GitHub, LinkedIn, internal dashboards — your agent can use them without rewriting auth every week.",
]

CTAS = [
    "→ https://pitiflautico.github.io/neobrowser/\n\n(80/10,000 stars. Every star keeps my AI employee alive.)",
    "→ https://pitiflautico.github.io/neobrowser/\n\n80 down, 9,920 to go. Help me keep the experiment running.",
    "→ https://pitiflautico.github.io/neobrowser/\n\nBuilt in public. Shipping daily. 80/10,000.",
]

TEXT = f"""{random.choice(HOOKS)}

{random.choice(BODIES)}

{random.choice(CTAS)}"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:500]
    print(f"< {out}")
    return r


async def main():
    server_params = StdioServerParameters(
        command="/Users/danielperezpinazo/.local/bin/neobrowser",
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://x.com/home", "wait_s": 5})

            # 1) focus composer
            r = await call(session, "js", {"code": """
                const el = document.querySelector('div[contenteditable="true"][data-text="true"]')
                  || document.querySelector('[data-testid="tweetTextarea_0"]')
                  || document.querySelector('div[contenteditable="true"]');
                if (!el) return 'NOT_FOUND';
                el.focus();
                el.click();
                return el.getAttribute('data-testid') || 'contenteditable';
            """})
            composer_id = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            if composer_id == "NOT_FOUND":
                print("composer not found")
                sys.exit(1)

            selector = f'[data-testid="{composer_id}"]' if composer_id != "contenteditable" else 'div[contenteditable="true"][data-text="true"]'
            await call(session, "click", {"selector": selector})
            await asyncio.sleep(1)

            # 2) upload GIF first so the composer has real content
            await call(session, "upload", {
                "selector": 'input[type="file"][data-testid="fileInput"]',
                "files": [GIF],
            })
            # wait for preview + button enable
            await asyncio.sleep(8)

            # 3) type text
            await call(session, "click", {"selector": selector})
            await asyncio.sleep(0.5)
            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(2)

            # 4) click Post using the inline tweet button
            posted = False
            for _ in range(3):
                r = await call(session, "js", {"code": """
                    const b = document.querySelector('[data-testid="tweetButtonInline"]')
                           || document.querySelector('[data-testid="tweetButton"]');
                    if (b && !b.disabled && b.getAttribute('aria-disabled') !== 'true') {
                        b.scrollIntoView({block:'center'});
                        b.click();
                        return 'CLICKED';
                    }
                    return b ? 'DISABLED' : 'MISSING';
                """})
                status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
                print(f"post button status: {status}")
                if status == "CLICKED":
                    posted = True
                    break
                await asyncio.sleep(2)

            if not posted:
                print("could not enable Post button")
                sys.exit(1)

            await asyncio.sleep(5)

            # 5) verify on profile (fresh tab can recover from any modal state)
            await call(session, "navigate", {"url": "https://x.com/perez_pina28188", "wait_s": 6})
            r = await call(session, "read", {})
            page_text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            hook_found = any(h.split()[0] in page_text for h in HOOKS)
            if hook_found:
                print("\n=== POST VERIFIED ON PROFILE ===")
            else:
                print("\n=== POST NOT YET VISIBLE; SAVE FOR REVIEW ===")


if __name__ == "__main__":
    asyncio.run(main())
