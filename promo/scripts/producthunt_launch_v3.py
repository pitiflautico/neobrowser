import asyncio
import os
import re
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

NAME = "NeoBrowser"
TAGLINE = "Your AI drives real Chrome — with your real logged-in sessions"
TOPICS = ["Developer Tools", "Open Source", "Artificial Intelligence"]
WEBSITE = "https://neobrowser.is-a.dev/"
GITHUB = "https://github.com/pitiflautico/neobrowser"

DESCRIPTION = """Hey Product Hunt 👋

I built NeoBrowser after watching every browser automation tool for AI fail the same way: fresh headless browser, no cookies, instant bot detection.

NeoBrowser drives your real Google Chrome via CDP — with your real logged-in sessions (opt-in), a genuine fingerprint (passes bot.sannysoft live in CI), human-cadence clicks, and first-class bot-wall detection so the agent reacts to CAPTCHAs instead of hallucinating success.

It's a single 6.4 MB Rust binary with 67 tools: multi-tab browsing, forms, upload/download, multi-source search, record/replay playbooks.

What I'm proudest of: the honest benchmark vs Playwright MCP in the repo. Playwright is faster; we do things it can't. Both get walled equally on adversarial pages. No hype.

Current bet: 89/10,000 GitHub stars, documented publicly at gentle-khapse-c58c79.netlify.app.

MIT licensed. Feedback welcome — especially from folks who've fought bot detection before."""

MAKER_COMMENT = "Maker here — happy to answer anything. Two technical rabbit holes if you're curious: (1) cross-platform Chrome cookie decryption (Keychain/secret-service/DPAPI) done safely and opt-in, (2) why genuine consistency beats spoof stacking for fingerprint checks. Both are in the codebase, MIT."

GALLERY = [
    os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-vs-headless.gif"),
    os.path.expanduser("~/.neobrowser/promo-home/downloads/demo.gif"),
    os.path.expanduser("~/.neobrowser/promo-home/downloads/hero-clip.mp4"),
]


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


async def js_set_value(session, selector, value):
    code = f"""
        const el = {selector};
        if (el) {{
            el.focus();
            el.value = {value!r};
            el.dispatchEvent(new Event('input', {{bubbles:true}}));
            el.dispatchEvent(new Event('change', {{bubbles:true}}));
            el.dispatchEvent(new KeyboardEvent('keydown', {{key:'a', bubbles:true}}));
            el.dispatchEvent(new KeyboardEvent('keyup', {{key:'a', bubbles:true}}));
            return 'SET_VALUE';
        }}
        return 'NOT_FOUND';
    """
    return await call(session, "js", {"code": code})


async def main():
    server_params = StdioServerParameters(
        command=os.path.expanduser("~/.local/bin/neobrowser"),
        args=[],
        env={
            **os.environ,
            "NEOBROWSER_HOME": NEO_HOME,
            "NEOBROWSER_REAL_PROFILE": PROFILE,
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "producthunt.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 10})
            await asyncio.sleep(3)

            # accept cookies
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /accept all/i.test(x.textContent));
                if (b) { b.click(); return 'ACCEPTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            # find URL input and type
            r = await call(session, "find", {"intent": "product url input"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
                await asyncio.sleep(1)
                await call(session, "type", {"text": WEBSITE, "human": True})
                await asyncio.sleep(2)
            else:
                await js_set_value(session, 'document.querySelector(\'input[name="url"]\')', WEBSITE)
                await asyncio.sleep(2)

            # click Get started
            r = await call(session, "find", {"intent": "get started button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            else:
                await call(session, "js", {"code": """
                    const btns = Array.from(document.querySelectorAll('button'));
                    const b = btns.find(x => /get started/i.test(x.textContent));
                    if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_GET_STARTED'; }
                    return 'NO_GET_STARTED_BUTTON';
                """})
            await asyncio.sleep(12)

            # detect URL rejection
            r = await call(session, "js", {"code": """
                const text = document.body.innerText;
                if (text.includes("can't hunt this product") || text.includes("seems to be invalid") || text.includes("URL is not")) return 'URL_REJECTED';
                return 'OK';
            """})
            check = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            if "URL_REJECTED" in check:
                print("\n=== PRODUCT HUNT REJECTED THE URL ===")
                return

            # read current page
            r = await call(session, "read", {})
            text = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content)
            print("\n--- FORM PAGE TEXT ---")
            print(text[:3000])

            # fill name
            await js_set_value(session, 'document.querySelector(\'input[name="name"]\') || document.querySelector(\'input[placeholder*="product name" i]\')', NAME)
            await asyncio.sleep(1)

            # fill tagline
            await js_set_value(session, 'document.querySelector(\'input[name="tagline"]\') || document.querySelector(\'input[placeholder*="tagline" i]\') || document.querySelector(\'input[maxlength="60"]\')', TAGLINE)
            await asyncio.sleep(1)

            # fill description
            await js_set_value(session, 'document.querySelector(\'textarea[name="description"]\') || document.querySelector(\'textarea[placeholder*="description" i]\')', DESCRIPTION)
            await asyncio.sleep(1)

            # fill website
            await js_set_value(session, 'document.querySelector(\'input[name="website_url"]\') || document.querySelector(\'input[placeholder*="website" i]\')', WEBSITE)
            await asyncio.sleep(1)

            # fill github
            await js_set_value(session, 'document.querySelector(\'input[name="github_url"]\') || document.querySelector(\'input[placeholder*="github" i]\')', GITHUB)
            await asyncio.sleep(1)

            # topics
            for topic in TOPICS:
                r = await call(session, "find", {"intent": "add topic button"})
                bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
                if bid:
                    await call(session, "click", {"backend_node_id": bid})
                await asyncio.sleep(1)
                await js_set_value(session, 'document.querySelector(\'input[placeholder*="topic" i]\') || document.querySelector(\'input[placeholder*="search" i]\')', topic)
                await asyncio.sleep(1)
                await call(session, "js", {"code": f"""
                    const opts = Array.from(document.querySelectorAll('li, div[role="option"]'));
                    const opt = opts.find(x => x.textContent.trim().toLowerCase() === {topic.lower()!r});
                    if (opt) {{ opt.click(); return 'SELECTED_TOPIC'; }}
                    return 'TOPIC_OPTION_NOT_FOUND';
                """})
                await asyncio.sleep(1)

            # gallery uploads
            for path in GALLERY:
                if not os.path.exists(path):
                    print(f"skip missing gallery file: {path}")
                    continue
                r = await call(session, "upload", {
                    "selector": 'input[type="file"]',
                    "files": [path],
                })
                await asyncio.sleep(8)

            # submit
            await asyncio.sleep(3)
            r = await call(session, "find", {"intent": "submit or launch button"})
            bid = extract_bid(" ".join(c.text if hasattr(c, "text") else str(c) for c in r.content))
            if bid:
                await call(session, "click", {"backend_node_id": bid})
            else:
                await call(session, "js", {"code": """
                    const btns = Array.from(document.querySelectorAll('button'));
                    const b = btns.find(x => /submit|launch|publish/i.test(x.textContent) && !x.disabled);
                    if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_SUBMIT'; }
                    return 'SUBMIT_NOT_FOUND';
                """})
            await asyncio.sleep(8)

            # add maker comment if post page loaded
            await js_set_value(session, 'document.querySelector(\'textarea[placeholder*="comment" i]\') || document.querySelector(\'textarea[name="comment"]\')', MAKER_COMMENT)
            r2 = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /post comment|submit comment/i.test(x.textContent));
                if (b) { b.click(); return 'COMMENT_POSTED'; }
                return 'COMMENT_BUTTON_NOT_FOUND';
            """})
            print("maker comment:", " ".join(c.text if hasattr(c, "text") else str(c) for c in r2.content).strip())

            print("\n=== PRODUCT HUNT LAUNCH ATTEMPT COMPLETE ===")


if __name__ == "__main__":
    asyncio.run(main())
