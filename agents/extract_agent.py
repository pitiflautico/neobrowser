#!/usr/bin/env python3
"""
Extract Agent — intelligent scraping via neobrowser-rs MCP.

Detects repeated DOM patterns, infers schema, paginates, exports JSON.

Usage:
    python3 extract_agent.py https://shop.com/products
    python3 extract_agent.py https://shop.com/products --max-pages 5
"""

import argparse
import asyncio
import json
import sys
import time
from urllib.parse import urlparse

sys.path.insert(0, "/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/ai-chat")
from aichat.neo_client import NeoClient

# JS: find the most repeated element selector (list detection)
JS_DETECT_LIST = """
(() => {
    const candidates = {};
    document.querySelectorAll('*[class]').forEach(el => {
        if (['SCRIPT','STYLE','META','LINK','HEAD','HTML','BODY','NAV','HEADER','FOOTER','NOSCRIPT','SVG'].includes(el.tagName)) return;
        if (el.children.length === 0 && (el.innerText||'').length < 10) return;
        // Use first class only for simpler selectors
        const firstClass = el.classList[0];
        if (!firstClass) return;
        const key = el.tagName.toLowerCase() + '.' + firstClass;
        if (!candidates[key]) candidates[key] = {els: [], selector: key};
        candidates[key].els.push(el);
    });
    let best = null, bestScore = 0;
    for (const [key, data] of Object.entries(candidates)) {
        const els = data.els;
        if (els.length < 3) continue;
        // Score: count + bonus for links/images inside
        const hasLinks = els.filter(e => e.querySelector('a[href]')).length;
        const hasImages = els.filter(e => e.querySelector('img')).length;
        const avgChildren = els.reduce((s,e) => s + e.children.length, 0) / els.length;
        // Prefer elements with structure (not bare text nodes)
        if (avgChildren < 1) continue;
        const score = els.length + hasLinks * 2 + hasImages + avgChildren;
        if (score > bestScore) {
            bestScore = score;
            best = data;
        }
    }
    if (!best) return JSON.stringify({found: false});
    return JSON.stringify({found: true, selector: best.selector, count: best.els.length});
})()
"""

# JS: infer schema from repeated elements
JS_INFER_SCHEMA = """
((selector) => {
    const els = [...document.querySelectorAll(selector)].slice(0, 5);
    if (!els.length) return JSON.stringify({fields: []});
    const fields = [];
    // Check for common field patterns
    const first = els[0];
    // Title: first heading or strong text
    const h = first.querySelector('h1,h2,h3,h4,h5,h6,[class*=title],[class*=name]');
    if (h) fields.push({name: 'title', query: h.tagName.toLowerCase() + (h.className ? '.' + [...h.classList].join('.') : ''), attr: 'innerText'});
    // Price
    const price = first.querySelector('[class*=price],[class*=cost],[data-price]');
    if (price) fields.push({name: 'price', query: price.tagName.toLowerCase() + (price.className ? '.' + [...price.classList].join('.') : ''), attr: 'innerText'});
    // Link
    const a = first.querySelector('a[href]');
    if (a) fields.push({name: 'link', query: 'a[href]', attr: 'href'});
    // Image
    const img = first.querySelector('img[src]');
    if (img) fields.push({name: 'image', query: 'img[src]', attr: 'src'});
    // Description: longest text block that isn't title/price
    const texts = [...first.querySelectorAll('p,span,div')].filter(e => {
        const t = e.innerText?.trim();
        return t && t.length > 20 && t.length < 500;
    });
    if (texts.length) {
        const desc = texts.sort((a,b) => b.innerText.length - a.innerText.length)[0];
        fields.push({name: 'description', query: desc.tagName.toLowerCase() + (desc.className ? '.' + [...desc.classList].join('.') : ''), attr: 'innerText'});
    }
    // Fallback: if no fields detected, grab all text
    if (!fields.length) fields.push({name: 'text', query: '*', attr: 'innerText'});
    return JSON.stringify({fields});
})
"""

# JS: extract items using detected selector + schema
JS_EXTRACT = """
((selector, fields) => {
    const els = [...document.querySelectorAll(selector)];
    const items = els.map(el => {
        const item = {};
        for (const f of fields) {
            const target = el.querySelector(f.query) || (el.matches(f.query) ? el : null);
            if (!target) continue;
            let val = f.attr === 'innerText' ? target.innerText?.trim()
                     : f.attr === 'href' ? target.href
                     : f.attr === 'src' ? target.src
                     : target.getAttribute(f.attr);
            if (val) item[f.name] = val;
        }
        return item;
    }).filter(i => Object.keys(i).length > 0);
    return JSON.stringify({count: items.length, items});
})
"""

# JS: detect pagination
JS_DETECT_PAGINATION = """
(() => {
    // Next button
    const nextPatterns = ['next', 'siguiente', 'suivant', '›', '»', '→', '>'];
    const links = [...document.querySelectorAll('a, button')];
    for (const el of links) {
        const text = (el.innerText || el.getAttribute('aria-label') || '').toLowerCase().trim();
        if (nextPatterns.some(p => text.includes(p)) && !el.disabled) {
            const label = el.innerText?.trim() || el.getAttribute('aria-label') || 'next';
            return JSON.stringify({type: 'next_button', label, disabled: false});
        }
    }
    // Page numbers
    const pageLinks = links.filter(el => /^\\d+$/.test(el.innerText?.trim()));
    if (pageLinks.length >= 2) {
        const nums = pageLinks.map(e => parseInt(e.innerText.trim())).sort((a,b) => a-b);
        return JSON.stringify({type: 'page_numbers', pages: nums});
    }
    // Infinite scroll hint (large scrollable area)
    const scrollable = document.documentElement.scrollHeight > window.innerHeight * 2;
    if (scrollable) return JSON.stringify({type: 'infinite_scroll'});
    return JSON.stringify({type: 'none'});
})()
"""


async def eval_js(neo: NeoClient, js: str) -> dict:
    """Eval JS and parse the result JSON."""
    raw = await neo.call_tool("browser_act", {"kind": "eval", "text": js})
    # Response is the full browser_act JSON — extract eval_result from effect field
    try:
        wrapper = json.loads(raw)
        text = wrapper.get("effect", raw)
    except (json.JSONDecodeError, TypeError):
        text = raw
    # Strip eval_result: prefix
    if isinstance(text, str) and text.startswith("eval_result: "):
        text = text[len("eval_result: "):]
    try:
        return json.loads(text)
    except (json.JSONDecodeError, TypeError):
        return {"raw": text}


async def run(url: str, max_pages: int = 10):
    domain = urlparse(url).netloc.replace("www.", "")
    ts = int(time.time())
    outfile = f"/tmp/extract_{domain.replace('.', '_')}_{ts}.json"

    neo = NeoClient()
    try:
        print(f"[*] Starting neobrowser...")
        await neo.start()

        print(f"[*] Opening {url}")
        await neo.call_tool("browser_open", {"url": url, "mode": "chrome"})
        await asyncio.sleep(2)  # let page render

        # 1. Detect list pattern
        print("[*] Detecting list patterns...")
        list_info = await eval_js(neo, JS_DETECT_LIST)
        if not list_info.get("found"):
            print("[!] No repeated element patterns found. Page may not be a list.")
            print("[*] Dumping page text as fallback...")
            page = await neo.observe("see")
            result = {"url": url, "type": "single_page", "content": page[:5000]}
            with open(outfile, "w") as f:
                json.dump(result, f, indent=2, ensure_ascii=False)
            print(f"[+] Saved to {outfile}")
            return

        selector = list_info["selector"]
        print(f"[+] Found list: {selector} ({list_info['count']} items)")

        # 2. Infer schema
        print("[*] Inferring schema...")
        schema = await eval_js(neo, f"({JS_INFER_SCHEMA})('{selector}')")
        fields = schema.get("fields", [])
        field_names = [f["name"] for f in fields]
        print(f"[+] Schema: {', '.join(field_names) or 'text only'}")

        # 3. Extract items across pages
        all_items = []
        pages_scraped = 0

        for page_num in range(1, max_pages + 1):
            print(f"[*] Extracting page {page_num}...")
            fields_json = json.dumps(fields).replace("'", "\\'")
            extract_js = f"({JS_EXTRACT})('{selector}', {json.dumps(fields)})"
            page_data = await eval_js(neo, extract_js)

            items = page_data.get("items", [])
            if not items:
                print(f"[*] No items on page {page_num}, stopping.")
                break

            # Deduplicate by first field value
            existing = {json.dumps(i, sort_keys=True) for i in all_items}
            new_items = [i for i in items if json.dumps(i, sort_keys=True) not in existing]

            if not new_items and page_num > 1:
                print(f"[*] No new items on page {page_num}, stopping.")
                break

            all_items.extend(new_items)
            pages_scraped = page_num
            print(f"    +{len(new_items)} items (total: {len(all_items)})")

            if page_num >= max_pages:
                break

            # 4. Paginate
            pag = await eval_js(neo, JS_DETECT_PAGINATION)
            pag_type = pag.get("type", "none")

            if pag_type == "next_button" and not pag.get("disabled"):
                label = pag["label"]
                print(f"[*] Clicking next: '{label}'")
                try:
                    await neo.call_tool("browser_act", {"kind": "click", "target": label})
                    await asyncio.sleep(2)
                except Exception as e:
                    print(f"[!] Click failed: {e}")
                    break
            elif pag_type == "page_numbers":
                next_num = page_num + 1
                if next_num in pag.get("pages", []):
                    print(f"[*] Clicking page {next_num}")
                    await neo.call_tool("browser_act", {"kind": "click", "target": str(next_num)})
                    await asyncio.sleep(2)
                else:
                    break
            elif pag_type == "infinite_scroll":
                print("[*] Scrolling down...")
                await neo.call_tool("browser_act", {"kind": "scroll", "direction": "down"})
                await asyncio.sleep(2)
            else:
                break

        # 5. Export
        result = {
            "url": url,
            "domain": domain,
            "timestamp": ts,
            "schema": field_names,
            "pages_scraped": pages_scraped,
            "total_items": len(all_items),
            "items": all_items,
        }

        with open(outfile, "w") as f:
            json.dump(result, f, indent=2, ensure_ascii=False)

        print(f"\n{'='*50}")
        print(f"  URL:     {url}")
        print(f"  Schema:  {', '.join(field_names)}")
        print(f"  Pages:   {pages_scraped}")
        print(f"  Items:   {len(all_items)}")
        print(f"  Output:  {outfile}")
        print(f"{'='*50}")

    finally:
        await neo.stop()


def main():
    parser = argparse.ArgumentParser(description="Extract Agent — scrape structured data from any list page")
    parser.add_argument("url", help="Target URL to scrape")
    parser.add_argument("--max-pages", type=int, default=10, help="Max pages to scrape (default: 10)")
    args = parser.parse_args()
    asyncio.run(run(args.url, args.max_pages))


if __name__ == "__main__":
    main()
