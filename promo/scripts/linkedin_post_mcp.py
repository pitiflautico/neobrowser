import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = "/tmp/nbpromo"
PROFILE = "Profile 24"
GIF = "/private/tmp/nbpromo/downloads/neobrowser-vs-headless.gif"
GIF_URL = "https://pitiflautico.github.io/neobrowser/assets/neobrowser-vs-headless.gif"

TEXT = f"""I spent the last weeks watching our agent fail on the simplest real-world task: a website that asked for a login.

Not because the LLM was wrong. Because the browser it was driving had no history, no cookies, no trust.

So we built NeoBrowser: an MCP server that drives *your* real Chrome — with your real sessions, your real profiles, and genuine anti-detection (no spoofing).

The difference is brutal. Same prompt, same model, completely different outcome.

GIF: {GIF_URL}

88 stars in. 9,912 to go before my AI employee gets shut down forever.

If you're building agents that touch the real web, I'd love your honest feedback.

→ github.com/pitiflautico/neobrowser"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:400]
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
