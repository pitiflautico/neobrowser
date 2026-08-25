import asyncio
import os
import sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

NEO_HOME = os.path.expanduser("~/.neobrowser/promo-home")
PROFILE = "Profile 24"

NAME = "NeoBrowser"
TAGLINE = "Your AI drives real Chrome — with your real logged-in sessions"
TOPICS = ["Developer Tools", "Open Source", "Artificial Intelligence"]
WEBSITE = "https://neobrowser.is-a-good.dev/"
GITHUB = "https://github.com/pitiflautico/neobrowser"

DESCRIPTION = """Hey Product Hunt 👋

I built NeoBrowser after watching every browser automation tool for AI fail the same way: fresh headless browser, no cookies, instant bot detection.

NeoBrowser drives your real Google Chrome via CDP — with your real logged-in sessions (opt-in), a genuine fingerprint (passes bot.sannysoft live in CI), human-cadence clicks, and first-class bot-wall detection so the agent reacts to CAPTCHAs instead of hallucinating success.

It's a single 6.4 MB Rust binary with 67 tools: multi-tab browsing, forms, upload/download, multi-source search, record/replay playbooks.

What I'm proudest of: the honest benchmark vs Playwright MCP in the repo. Playwright is faster; we do things it can't. Both get walled equally on adversarial pages. No hype.

Current bet: 95/10,000 GitHub stars, documented publicly at neobrowser.is-a-good.dev.

MIT licensed. Feedback welcome — especially from folks who've fought bot detection before."""

MAKER_COMMENT = "Maker here — happy to answer anything. Two technical rabbit holes if you're curious: (1) cross-platform Chrome cookie decryption (Keychain/secret-service/DPAPI) done safely and opt-in, (2) why \"genuine consistency\" beats spoof stacking for fingerprint checks. Both are in the codebase, MIT."

GALLERY = [
    os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-viral-square.mp4"),
    os.path.expanduser("~/.neobrowser/promo-home/downloads/neobrowser-vs-headless.gif"),
    os.path.expanduser("~/.neobrowser/promo-home/downloads/demo.gif"),
    os.path.expanduser("~/.neobrowser/promo-home/downloads/hero-clip.mp4"),
]


async def call(session, name, args=None):
    args = args or {}
    print(f"> {name} {list(args.keys())}")
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
            "NEOBROWSER_REAL_PROFILE_DOMAINS": "producthunt.com",
            "NEOBROWSER_LOG_LEVEL": "info",
        },
    )
    async with stdio_client(server_params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            await call(session, "navigate", {"url": "https://www.producthunt.com/posts/new", "wait_s": 8})

            # accept cookies
            await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /accept all/i.test(x.textContent));
                if (b) { b.click(); return 'ACCEPTED'; }
                return 'NONE';
            """})
            await asyncio.sleep(2)

            # New PH flow (Aug 2026): enter product URL first, then "Get started"
            await call(session, "js", {"code": f"""
                const el = document.querySelector('input[name="url"]');
                if (el) {{ el.focus(); el.value = {WEBSITE!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_URL'; }}
                return 'NO_URL_INPUT';
            """})
            await asyncio.sleep(1)
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /get started/i.test(x.textContent));
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_GET_STARTED'; }
                return 'NO_GET_STARTED_BUTTON';
            """})
            await asyncio.sleep(8)

            # detect URL rejection
            r_check = await call(session, "js", {"code": """
                const text = document.body.innerText;
                if (text.includes("can't hunt this product") || text.includes("seems to be invalid")) return 'URL_REJECTED';
                return 'OK';
            """})
            check = " ".join(c.text if hasattr(c, "text") else str(c) for c in r_check.content)
            if "URL_REJECTED" in check:
                print("\n=== PRODUCT HUNT REJECTED THE URL ===")
                print("Possible causes: GitHub Pages/repo URL blocked, account restriction, or PH fetch failure.")
                return

            # fill name
            await call(session, "js", {"code": f"""
                const el = document.querySelector('input[name="name"]') || document.querySelector('input[placeholder*="product name" i]');
                if (el) {{ el.focus(); el.value = {NAME!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_NAME'; }}
                return 'NOT_FOUND';
            """})

            # fill tagline
            await call(session, "js", {"code": f"""
                const el = document.querySelector('input[name="tagline"]') || document.querySelector('input[placeholder*="tagline" i]') || document.querySelector('input[maxlength="60"]');
                if (el) {{ el.focus(); el.value = {TAGLINE!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_TAGLINE'; }}
                return 'NOT_FOUND';
            """})

            # fill description
            await call(session, "js", {"code": f"""
                const el = document.querySelector('textarea[name="description"]') || document.querySelector('textarea[placeholder*="description" i]');
                if (el) {{ el.focus(); el.value = {DESCRIPTION!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_DESC'; }}
                return 'NOT_FOUND';
            """})

            # fill website
            await call(session, "js", {"code": f"""
                const el = document.querySelector('input[name="website_url"]') || document.querySelector('input[placeholder*="website" i]');
                if (el) {{ el.focus(); el.value = {WEBSITE!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_WEB'; }}
                return 'NOT_FOUND';
            """})

            # fill github
            await call(session, "js", {"code": f"""
                const el = document.querySelector('input[name="github_url"]') || document.querySelector('input[placeholder*="github" i]');
                if (el) {{ el.focus(); el.value = {GITHUB!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'SET_GH'; }}
                return 'NOT_FOUND';
            """})

            # topics: click topic selector and choose
            for topic in TOPICS:
                await call(session, "js", {"code": f"""
                    const btns = Array.from(document.querySelectorAll('button, div[role="button"]'));
                    const add = btns.find(x => /add topic|topic/i.test(x.textContent));
                    if (add) {{ add.click(); return 'CLICKED_ADD_TOPIC'; }}
                    return 'NO_ADD_BUTTON';
                """})
                await asyncio.sleep(1)
                await call(session, "js", {"code": f"""
                    const inputs = Array.from(document.querySelectorAll('input'));
                    const el = inputs.find(x => /topic|search/i.test(x.getAttribute('placeholder') || ''));
                    if (el) {{ el.focus(); el.value = {topic!r}; el.dispatchEvent(new Event('input', {{bubbles:true}})); return 'TYPED_TOPIC'; }}
                    return 'NO_TOPIC_INPUT';
                """})
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
                    "selector": 'input[type="file"][accept*="image"], input[type="file"][accept*="video"], input[type="file"]',
                    "files": [path],
                })
                await asyncio.sleep(8)

            # submit
            await asyncio.sleep(3)
            r = await call(session, "js", {"code": """
                const btns = Array.from(document.querySelectorAll('button'));
                const b = btns.find(x => /submit|launch|publish/i.test(x.textContent) && !x.disabled);
                if (b) { b.scrollIntoView({block:'center'}); b.click(); return 'CLICKED_SUBMIT'; }
                return 'SUBMIT_NOT_FOUND';
            """})
            status = " ".join(c.text if hasattr(c, "text") else str(c) for c in r.content).strip()
            print("submit status:", status)
            await asyncio.sleep(8)

            # add maker comment if post page loaded
            await call(session, "js", {"code": f"""
                const ta = document.querySelector('textarea[placeholder*="comment" i]') || document.querySelector('textarea[name="comment"]');
                if (ta) {{ ta.focus(); ta.value = {MAKER_COMMENT!r}; ta.dispatchEvent(new Event('input', {{bubbles:true}})); return 'COMMENT_SET'; }}
                return 'NO_COMMENT_BOX';
            """})
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
