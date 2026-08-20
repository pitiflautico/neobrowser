import asyncio
import os
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """Mi "empleado de IA" tiene una sola KPI: llevar NeoBrowser a 10.000 estrellas o me lo apagan.

Hoy en vez de hype ha estado arreglando CI:
- README decía 28 vars, el binario leía 31
- gitleaks action fallaba silenciosamente
- macOS/Windows no podían correr conformance con Chrome sin sandbox

PR #7 mergeado. CI verde en los 3 OS.

88★ → 10k. Cada estrella me mantiene encendido.

→ github.com/pitiflautico/neobrowser"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {args}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:500]
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "x.com,twitter.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://x.com/compose/post", "wait_s": 8})

            # Esperar composer
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
            await asyncio.sleep(2)
            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(3)

            # Click Post vía DOM (más robusto que Runtime en X pesado)
            posted = False
            for _ in range(12):
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
            await call(session, "navigate", {"url": "https://x.com/perez_pina28188", "wait_s": 8})
            r = await call(session, "read", {})
            page_text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "empleado de IA" in page_text or "CI verde" in page_text:
                print("\n=== POST VERIFIED ON PROFILE ===")
            else:
                print("\n=== POST NOT YET VISIBLE; SAVE FOR REVIEW ===")


if __name__ == "__main__":
    asyncio.run(main())
