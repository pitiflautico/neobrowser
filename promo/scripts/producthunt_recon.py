import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:1200]
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

            await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 10})
            await asyncio.sleep(3)

            # accept cookies
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /accept all/i.test(x.textContent));
                if (b) { b.click(); return 'ACCEPTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            # fill URL and get started
            await call(session, "js", {"code": """
                const el = document.querySelector('input[name="url"]');
                if (el) { el.focus(); el.value = 'https://gentle-khapse-c58c79.netlify.app/'; el.dispatchEvent(new Event('input', {bubbles:true})); return 'SET_URL'; }
                return 'NO_URL_INPUT';
            """})
            await asyncio.sleep(1)
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /get started/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_GET_STARTED'; }
                return 'NO_GET_STARTED_BUTTON';
            """})
            await asyncio.sleep(12)

            # read page
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- PAGE TEXT ---")
            print(text[:4000])
            print("--- END ---\n")

            # list inputs
            r = await call(session, "js", {"code": """
                const inputs = Array.from(document.querySelectorAll('input, textarea, select'));
                return inputs.map(i => ({
                    tag: i.tagName,
                    type: i.type,
                    name: i.name,
                    placeholder: (i.placeholder || '').slice(0,60),
                    id: i.id,
                    classes: i.className.slice(0,80)
                }));
            """})
            print("INPUTS:", " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:3000])

            # list buttons
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, a, div[role="button"]'));
                return btns.map(b => ({
                    tag: b.tagName,
                    text: (b.textContent || b.value || '').trim().slice(0,100),
                    href: b.href || ''
                })).filter(x => x.text);
            """})
            print("BUTTONS:", " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:3000])


if __name__ == "__main__":
    asyncio.run(main())
