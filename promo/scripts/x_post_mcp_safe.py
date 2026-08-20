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
    "→ https://pitiflautico.github.io/neobrowser/\n\n(88/10,000 stars. Every star keeps my AI employee alive.)",
    "→ https://pitiflautico.github.io/neobrowser/\n\n88 down, 9,912 to go. Help me keep the experiment running.",
    "→ https://pitiflautico.github.io/neobrowser/\n\nBuilt in public. Shipping daily. 88/10,000.",
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "x.com,twitter.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            # Use the direct compose URL to avoid home-feed noise.
            await call(session, "navigate", {"url": "https://x.com/compose/post", "wait_s": 8})

            # Wait for the composer to exist.
            for attempt in range(10):
                r = await call(session, "js", {"code": """
                    const el = document.querySelector('div[contenteditable="true"][data-text="true"]')
                      || document.querySelector('[data-testid="tweetTextarea_0"]')
                      || document.querySelector('div[contenteditable="true"]');
                    return el ? (el.getAttribute('data-testid') || 'contenteditable') : 'NOT_FOUND';
                """})
                composer_id = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
                if composer_id != "NOT_FOUND":
                    break
                await asyncio.sleep(2)

            if composer_id == "NOT_FOUND":
                print("composer not found")
                sys.exit(1)

            selector = f'[data-testid="{composer_id}"]' if composer_id != "contenteditable" else 'div[contenteditable="true"][data-text="true"]'
            await call(session, "click", {"selector": selector})
            await asyncio.sleep(3)

            # Upload GIF first.
            await call(session, "upload", {
                "selector": 'input[type="file"][data-testid="fileInput"]',
                "files": [GIF],
            })
            await asyncio.sleep(12)

            # Type text slowly.
            await call(session, "click", {"selector": selector})
            await asyncio.sleep(1)
            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(4)

            # Wait for the Post button to be enabled, then click.
            posted = False
            for _ in range(10):
                r = await call(session, "js", {"code": """
                    const b = document.querySelector('[data-testid="tweetButtonInline"]')
                           || document.querySelector('[data-testid="tweetButton"]');
                    if (b) {
                        const disabled = b.disabled || b.getAttribute('aria-disabled') === 'true';
                        return JSON.stringify({found:true, disabled, text: b.innerText.trim()});
                    }
                    return JSON.stringify({found:false});
                """})
                status_json = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
                try:
                    import json
                    status = json.loads(status_json)
                except Exception:
                    status = {}
                print("post button status:", status)
                if status.get("found") and not status.get("disabled"):
                    r2 = await call(session, "js", {"code": """
                        const b = document.querySelector('[data-testid="tweetButtonInline"]')
                               || document.querySelector('[data-testid="tweetButton"]');
                        if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED'; }
                        return 'MISSING';
                    """})
                    click_status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r2.content).strip()
                    if click_status == "CLICKED":
                        posted = True
                        break
                await asyncio.sleep(2)

            if not posted:
                print("could not enable/click Post button")
                sys.exit(1)

            await asyncio.sleep(8)

            # Verify on profile.
            await call(session, "navigate", {"url": "https://x.com/perez_pina28188", "wait_s": 8})
            r = await call(session, "read", {})
            page_text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            hook_found = any(h.split()[0] in page_text for h in HOOKS)
            if hook_found:
                print("\n=== POST VERIFIED ON PROFILE ===")
            else:
                print("\n=== POST NOT YET VISIBLE; SAVE FOR REVIEW ===")


if __name__ == "__main__":
    asyncio.run(main())
