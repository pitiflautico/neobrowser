import asyncio
import os
import re
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


def extract_bid(text):
    m = re.search(r'"backend_node_id"\s*:\s*(\d+)', text)
    if m:
        return int(m.group(1))
    return None


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

            await call(session, "navigate", {"url": "https://www.linkedin.com/feed/", "wait_s": 8})
            await asyncio.sleep(4)

            # find and click start a post
            r = await call(session, "find", {"intent": "start a post button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            else:
                await call(session, "js", {"code": """
                    const btns = Array.from(document.querySelectorAll('button, span, div'));
                    const b = btns.find(x => /crear publicaci|start a post|new post/i.test(x.textContent));
                    if (b) { b.click(); return 'CLICKED'; }
                    return 'NOT_FOUND';
                """})
            await asyncio.sleep(4)

            # find editor
            r = await call(session, "find", {"intent": "post text editor"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            else:
                await call(session, "js", {"code": """
                    const editor = document.querySelector('[contenteditable="true"]')
                               || document.querySelector('[role="textbox"]')
                               || document.querySelector('.ql-editor');
                    if (!editor) return 'NOT_FOUND';
                    editor.focus(); editor.click();
                    return 'EDITOR';
                """})
            await asyncio.sleep(1)

            await call(session, "type", {"text": TEXT, "human": True})
            await asyncio.sleep(3)

            # find post button
            r = await call(session, "find", {"intent": "post or publish button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            else:
                await call(session, "js", {"code": """
                    const btns = Array.from(document.querySelectorAll('button, span, div'));
                    const b = btns.find(x => /publicar|post/i.test(x.textContent) && !x.disabled);
                    if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED'; }
                    return 'NOT_FOUND';
                """})
            await asyncio.sleep(6)

            # verify
            await call(session, "navigate", {"url": "https://www.linkedin.com/in/me/recent-activity/all/", "wait_s": 6})
            await asyncio.sleep(4)
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "distribution is harder" in text or "10,000 GitHub stars" in text:
                print("\n=== LINKEDIN POST VERIFIED ===")
            else:
                print("\n=== LINKEDIN POST NOT VERIFIED ===")


if __name__ == "__main__":
    asyncio.run(main())
