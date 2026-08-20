import asyncio
import os
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

# Mirrors the selectors in producthunt_launch.py
CHECKS = {
    "name": 'input[name="name"], input[placeholder*="product name" i]',
    "tagline": 'input[name="tagline"], input[placeholder*="tagline" i], input[maxlength="60"]',
    "description": 'textarea[name="description"], textarea[placeholder*="description" i]',
    "website": 'input[name="website_url"], input[placeholder*="website" i]',
    "github": 'input[name="github_url"], input[placeholder*="github" i]',
    "topic_add_button": 'button, div[role="button"]',
    "file_input": 'input[type="file"]',
    "submit_button": 'button',
}


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "producthunt.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 8})

            # accept cookies if present
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /accept all/i.test(x.textContent));
                if (b) { b.click(); return 'ACCEPTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            # New PH flow (Aug 2026): enter product URL first, then "Get started"
            await call(session, "js", {"code": """
                const el = document.querySelector('input[name="url"]');
                if (el) { el.focus(); el.value = 'https://pitiflautico.github.io/neobrowser/'; el.dispatchEvent(new Event('input', {bubbles:true})); return 'SET_URL'; }
                return 'NO_URL_INPUT';
            """})
            await asyncio.sleep(1)
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /get started/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_GET_STARTED'; }
                return 'NO_GET_STARTED_BUTTON';
            """})
            await asyncio.sleep(8)

            results = {}
            for field, selector in CHECKS.items():
                if field == "topic_add_button":
                    r = await call(session, "js", {"code": f"""
                        const btns = Array.from(document.querySelectorAll('{selector}'));
                        const b = btns.find(x => /add topic|topic/i.test(x.textContent));
                        return b ? 'FOUND: ' + b.textContent.trim().slice(0,40) : 'NOT_FOUND';
                    """})
                elif field == "submit_button":
                    r = await call(session, "js", {"code": f"""
                        const btns = Array.from(document.querySelectorAll('{selector}'));
                        const b = btns.find(x => /submit|launch|publish/i.test(x.textContent) && !x.disabled);
                        return b ? 'FOUND: ' + b.textContent.trim().slice(0,40) : 'NOT_FOUND';
                    """})
                elif field == "file_input":
                    r = await call(session, "js", {"code": f"""
                        const el = document.querySelector('{selector}');
                        return el ? 'FOUND' : 'NOT_FOUND';
                    """})
                else:
                    r = await call(session, "js", {"code": f"""
                        const el = document.querySelector('{selector}');
                        return el ? 'FOUND: ' + (el.getAttribute('name') || el.getAttribute('placeholder') || el.tagName).slice(0,40) : 'NOT_FOUND';
                    """})
                out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
                results[field] = out
                await asyncio.sleep(0.5)

            print("\n=== DRY-RUN RESULTS ===")
            all_ok = True
            for field, result in results.items():
                ok = (result.startswith("FOUND:") or result == "NONE")
                if not ok:
                    all_ok = False
                print(f"{field}: {'OK' if ok else 'MISSING'} ({result[:60]})")

            if all_ok:
                print("\n=== PRODUCT HUNT FORM LOOKS READY ===")
            else:
                print("\n=== PRODUCT HUNT FORM HAS MISSING FIELDS ===")
                sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
