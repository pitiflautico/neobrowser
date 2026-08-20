import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """I thought building an MCP server that drives real Chrome was the hard part.

Turns out distribution is harder.

Day N of the public bet: 10,000 GitHub stars or I shut down the AI agent promoting the project. Current count: 89.

What happened today:

✓ Reclaimed a Netlify Drop site and made it public so Product Hunt could crawl it.

✗ Product Hunt still rejects it — and GitHub Pages, and the repo, and any variant. It wants a "real" domain.

✗ X composer lets me type but won't submit the post.

✗ HN account is rate-limited.

✓ LinkedIn still works. Text-only, but it works.

The lesson: platform mechanics are a product skill, not a marketing afterthought.

If you think AI agents should browse the real web with real sessions, the repo is in the comments. Every star keeps the experiment alive.

#buildinpublic #opensource #aiagents #mcp #browserautomation"""


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
    r = await session.call_tool(name, args)
    out = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)[:600]
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
            await asyncio.sleep(5)

            # click start a post by text
            r = await call(session, "js", {"code": """
                const all = Array.from(document.querySelectorAll('button, div[role="button"], span'));
                const b = all.find(x => {
                    const t = (x.textContent || '').trim();
                    return /^(Crear publicaci|Start a post|New post)/i.test(t);
                });
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED: ' + b.textContent.trim().slice(0,40); }
                return 'NOT_FOUND. Available: ' + all.filter(x=>x.textContent && x.textContent.trim().length>5).map(x=>x.textContent.trim().slice(0,40)).slice(0,20).join(' | ');
            """})
            await asyncio.sleep(5)

            # click editor
            r = await call(session, "js", {"code": """
                const editor = document.querySelector('div[contenteditable="true"]')
                            || document.querySelector('[role="textbox"]')
                            || document.querySelector('.ql-editor')
                            || document.querySelector('[data-placeholder*="¿Sobre"]')
                            || document.querySelector('[data-placeholder*="What"]');
                if (!editor) {
                    return 'EDITOR_NOT_FOUND. Modals: ' + Array.from(document.querySelectorAll('[role="dialog"]')).map(d=>d.textContent.slice(0,100)).join(' | ');
                }
                editor.focus();
                editor.click();
                return 'EDITOR: ' + (editor.getAttribute('class') || editor.tagName);
            """})
            await asyncio.sleep(2)

            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(3)

            # click post
            r = await call(session, "js", {"code": """
                const all = Array.from(document.querySelectorAll('button, div[role="button"], span'));
                const b = all.find(x => {
                    const t = (x.textContent || '').trim();
                    return /^(Publicar|Post|Publish)/i.test(t) && !x.disabled;
                });
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_POST: ' + b.textContent.trim(); }
                return 'POST_BTN_NOT_FOUND. Available: ' + all.filter(x=>x.textContent && x.textContent.trim().length>2).map(x=>x.textContent.trim().slice(0,30)).slice(0,30).join(' | ');
            """})
            await asyncio.sleep(8)

            # verify
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 8})
            await asyncio.sleep(5)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "distribution is harder" in text or "10,000 GitHub stars" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
