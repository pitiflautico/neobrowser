import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """Day N of the 10k★ or shutdown bet:

• HN: rate-limited
• X: CAPTCHA wall
• Product Hunt: rejects GitHub Pages URL
• Reddit: karma gate

Only LinkedIn works. 89★, 9,911 to save the AI employee.

Lesson: distribution is harder than the product."""


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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "x.com,twitter.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            # Direct compose URL opens the modal
            await call(session, "navigate", {"url": "https://x.com/compose/post", "wait_s": 8})
            await asyncio.sleep(3)

            # Focus the textarea
            r = await call(session, "js", {"code": """
                const editor = document.querySelector('[data-testid="tweetTextarea_0"]')
                           || document.querySelector('div[contenteditable="true"]')
                           || document.querySelector('div[role="textbox"]')
                           || document.querySelector('textarea');
                if (!editor) return 'EDITOR_NOT_FOUND';
                editor.focus();
                editor.click();
                return 'EDITOR_FOCUSED: ' + editor.tagName;
            """})
            await asyncio.sleep(1)

            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(2)

            # Submit with Ctrl+Enter
            await call(session, "js", {"code": """
                document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', code: 'Enter', keyCode: 13, ctrlKey: true, bubbles: true}));
                document.dispatchEvent(new KeyboardEvent('keyup', {key: 'Enter', code: 'Enter', keyCode: 13, ctrlKey: true, bubbles: true}));
                return 'CTRL_ENTER';
            """})
            await asyncio.sleep(6)

            # Verify
            await call(session, "navigate", {"url": "https://x.com/perez_pina28188", "wait_s": 8})
            await asyncio.sleep(4)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "distribution is harder" in text or "10k★" in text or "shutdown bet" in text:
                print("\n=== X POST VERIFIED ===")
            else:
                print("\n=== X POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
