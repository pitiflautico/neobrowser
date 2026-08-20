import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
GIF = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-viral-square.gif")

TEXT = """I built an MCP server that drives real Chrome because I was tired of watching agents fail at the one thing that should be easy: using a website I already use every day.

The problem isn't the LLM. It's that most browser tools hand the agent a sterile, headless browser with no history, no trust, and no session.

So I went the other way. NeoBrowser drives *your* Chrome, with *your* real sessions. The fingerprint is genuine because it literally is your browser.

The GIF below is the honest state of the mission: 88 stars in, 9,912 to go. Every star keeps the experiment alive.

If you think AI agents should use the real web like humans do, I'd love your feedback (or just a star to keep my AI employee off the chopping block).

→ github.com/pitiflautico/neobrowser"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args.get('url','')} {args.get('selector','')[:40]}")
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "linkedin.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.linkedin.com/feed/", "wait_s": 5})

            # click "Crear publicación" / "Start a post"
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, span'));
                const b = btns.find(x => /crear publicaci|start a post|new post/i.test(x.textContent));
                if (b) { b.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(3)

            # try to upload the GIF as an image
            await call(session, "upload", {
                "selector": 'input[type="file"][accept*="image"]',
                "files": [GIF],
            })
            await asyncio.sleep(6)

            # focus editor and type
            r = await call(session, "js", {"code": """
                const editor = document.querySelector('[contenteditable="true"]')
                           || document.querySelector('[role="textbox"]')
                           || document.querySelector('.ql-editor')
                           || document.querySelector('[data-placeholder*="¿Sobre"]')
                           || document.querySelector('[data-placeholder*="What"]');
                if (!editor) return 'NOT_FOUND';
                editor.focus();
                editor.click();
                return editor.getAttribute('class') || 'EDITOR';
            """})
            await asyncio.sleep(1)
            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(2)

            # click Publicar / Post
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, span'));
                const b = btns.find(x => /publicar|post/i.test(x.textContent) && !x.disabled);
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("post status:", status)

            await asyncio.sleep(5)

            # verify in recent activity
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 5})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "real Chrome" in text or "NeoBrowser" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
