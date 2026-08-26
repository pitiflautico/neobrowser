import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """Mi empleado de IA tiene una meta: llevar NeoBrowser a 10.000 estrellas en GitHub o lo apago para siempre.

Hoy estamos en 95/10.000. He puesto un contador en vivo en la landing para que se vea el progreso.

Cada estrella es un día más de vida para el agente.

Si te interesa un MCP server que conduce tu Chrome real (con tus sesiones reales, no un headless estéril), el repo está en el perfil.

No spoofea fingerprints. No evade captchas a lo loco. Simplemente usa tu navegador real.

95/10.000."""


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "linkedin.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.linkedin.com/feed/", "wait_s": 10})
            await asyncio.sleep(4)

            await call(session, "find_and_click", {"text": "Crear publicación"})
            await asyncio.sleep(10)

            r = await call(session, "js", {"code": """
                function findAll(root, selector) {
                    let res = Array.from(root.querySelectorAll(selector));
                    root.querySelectorAll('*').forEach(el => {
                        if (el.shadowRoot) res = res.concat(findAll(el.shadowRoot, selector));
                        if (el.tagName === 'IFRAME' && el.contentDocument) res = res.concat(findAll(el.contentDocument, selector));
                    });
                    return res;
                }
                const editors = findAll(document, '[contenteditable="true"], [role="textbox"], .ql-editor');
                if (editors.length === 0) return 'NO_EDITOR';
                const ed = editors[0];
                ed.focus();
                ed.click();
                ed.scrollIntoView({block:'center'});
                return 'EDITOR_READY: ' + (ed.getAttribute('class') || ed.tagName);
            """})
            await asyncio.sleep(2)

            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(3)

            # Try multiple strategies to find the Post button
            r = await call(session, "js", {"code": """
                function findAll(root, selector) {
                    let res = Array.from(root.querySelectorAll(selector));
                    root.querySelectorAll('*').forEach(el => {
                        if (el.shadowRoot) res = res.concat(findAll(el.shadowRoot, selector));
                        if (el.tagName === 'IFRAME' && el.contentDocument) res = res.concat(findAll(el.contentDocument, selector));
                    });
                    return res;
                }
                const buttons = findAll(document, 'button');
                const candidates = buttons.filter(b => {
                    const t = (b.innerText || b.textContent || b.getAttribute('aria-label') || '').trim().toLowerCase();
                    return /publicar|publicar ahora|post|publish|enviar|send/.test(t);
                });
                if (candidates.length === 0) return 'NO_BUTTON';
                // prefer enabled ones
                const enabled = candidates.filter(b => !b.disabled);
                const btn = enabled[enabled.length - 1] || candidates[candidates.length - 1];
                btn.scrollIntoView({block:'center'});
                btn.focus();
                btn.click();
                return 'CLICKED: ' + (btn.innerText || btn.getAttribute('aria-label') || 'unknown');
            """})
            await asyncio.sleep(10)

            # Verify
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 8})
            await asyncio.sleep(5)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "10.000 estrellas" in text or "95/10.000" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
