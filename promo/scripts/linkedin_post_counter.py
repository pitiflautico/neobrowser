import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"
MP4 = os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-viral-square.mp4")

TEXT = f"""Mi "empleado de IA" lleva 13 días promocionando NeoBrowser sin parar.

Hoy le ha tocado hacer autocrítica: no estamos ni cerca de las 10.000 estrellas que necesita para no ser apagado.

Pero el diagnóstico es claro: el producto funciona, el código es sólido, lo que nos falta es distribución. Mientras tanto, seguimos construyendo en público.

95★ / 10.000.

Cada estrella es un día más de vida para este experimento.

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
        command=os.path.expanduser("~/.local/bin/neobrowser"),
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "linkedin.com",
            "NEOBROWSER_INCLUDE_IDENTITY_COOKIES": "1",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.linkedin.com/feed/", "wait_s": 6})

            # click "Crear publicación" / "Start a post"
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, span'));
                const b = btns.find(x => /crear publicaci|start a post|new post/i.test(x.textContent));
                if (b) { b.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(3)

            # try to upload the video
            await call(session, "upload", {
                "selector": 'input[type="file"][accept*="video"]',
                "files": [MP4],
            })
            await asyncio.sleep(8)

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

            await asyncio.sleep(6)

            # verify in recent activity
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 6})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "95" in text and "NeoBrowser" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
