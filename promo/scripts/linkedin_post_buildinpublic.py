import asyncio
import os
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

TEXT = """I thought building an MCP server that drives real Chrome was the hard part.

Turns out distribution is harder.

Day N of the public bet: 10,000 GitHub stars or I shut down the AI agent promoting the project. Current count: 89.

Here's what happened this week:

✓ Hacker News launch worked — 35 stars in a few hours, great technical feedback, even bug reports that made the product better.

✗ HN now rate-limits the account (comment-toofast). Can't comment there for a while.

✗ X hit a CAPTCHA on the account. Can't post.

✗ Product Hunt rejects both the GitHub Pages URL and the GitHub repo URL with "can't hunt this product." Need to figure that out before Tuesday.

✗ Reddit r/selfhosted swallowed the post without publishing it — karma gate or spam filter.

✓ LinkedIn still works. Text-only, but it works.

The uncomfortable truth: you can have a working product, benchmarks, demos, and a story, and still get stuck on platform mechanics.

What I'm doing next:
1. Fix the Product Hunt URL issue (probably need a custom domain or manual verification).
2. Keep publishing honest, useful content on LinkedIn.
3. Reach out directly to people who care about real-session browser automation — no mass pitches, just genuine conversations.

If you think AI agents should use the real web like humans do, the repo is in the comments. Every star extends the experiment.

#buildinpublic #opensource #aiagents #mcp #browserautomation"""


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

            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, span, div'));
                const b = btns.find(x => /crear publicaci|start a post|new post/i.test(x.textContent));
                if (b) { b.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            await asyncio.sleep(3)

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

            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button, span, div'));
                const b = btns.find(x => /publicar|post/i.test(x.textContent) && !x.disabled);
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED'; }
                return 'NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("post status:", status)

            await asyncio.sleep(5)

            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 5})
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "distribution is harder" in text or "10,000 GitHub stars" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
