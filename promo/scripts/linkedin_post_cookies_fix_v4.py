import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """Me ha vuelto a paso.

Un usuario me escribió diciendo que NeoBrowser le deslogueaba de Gmail cuando lo usaba. Primera reacción: imposible, no tocamos su perfil de Chrome.

Pero claro, el problema no era que escribiéramos en su perfil. Era que importábamos demasiadas cookies reales a un navegador fantasma, y Google detectaba la sesión clonada.

Me tiré un rato mirando la base de datos de cookies de Chrome. Había tokens de Gmail (GMAIL_AT, OSID) y cookies de fingerprinting (AEC, SOCS, 1P_JAR) que no estábamos filtrando. Cada vez que el agente las usaba desde un headless, Google marcaba la sesión original como sospechosa y la mataba.

El fix: ampliar la lista negra de cookies de identidad y añadir una categoría de "fingerprint cookies" de alto riesgo. Ahora se excluyen por defecto. Tests incluidos.

Lección que me queda: en un browser agent, "más sesión real" no siempre es "más indetectable". Las plataformas ven la inconsistencia entre navegadores.

Si te interesa cómo se hace esto sin ser evil, el repo está en el perfil.

95/10.000. Cada estrella es un respiro."""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
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

            # Open post composer
            await call(session, "find_and_click", {"text": "Crear publicación"})
            await asyncio.sleep(10)

            # Focus the first rich text editor inside the modal
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

            # Type the post
            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(3)

            # Click Post button
            await call(session, "find_and_click", {"text": "Publicar"})
            await asyncio.sleep(10)

            # Verify
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 8})
            await asyncio.sleep(5)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "deslogueaba de Gmail" in text or "GMAIL_AT" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
