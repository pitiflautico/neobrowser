#!/usr/bin/env python3
"""
eDreams hotel search module — handles the React SPA form interaction.

Usage:
    from edreams_search import search_edreams
    hotels = await search_edreams(neo, "Alicante", "2026-03-20", "2026-03-22")
"""
import asyncio
import json
import re


async def eval_js(neo, js):
    raw = await neo.call_tool("browser_act", {"kind": "eval", "text": js})
    try:
        wrapper = json.loads(raw)
        text = wrapper.get("effect", raw)
    except (json.JSONDecodeError, TypeError):
        text = raw
    if isinstance(text, str) and text.startswith("eval_result: "):
        text = text[len("eval_result: "):]
    try:
        return json.loads(text)
    except (json.JSONDecodeError, TypeError):
        return {"raw": text}


async def search_edreams(neo, destination: str, checkin: str, checkout: str, spa_filter: bool = True) -> list[dict]:
    """
    Search eDreams for hotels. Returns list of {name, price, rating, location, link}.

    Steps:
    1. Open eDreams hotel search page
    2. Fill destination via React-compatible JS (nativeInputValueSetter)
    3. Select autocomplete suggestion
    4. Set dates via calendar clicks
    5. Click search
    6. Optionally activate SPA filter
    7. Extract results from see output (most reliable)
    """
    print(f"  [eDreams] Opening search page...")
    await neo.call_tool("browser_open", {"url": "https://www.edreams.es/hoteles/", "mode": "chrome"})
    await asyncio.sleep(4)

    # Accept cookies
    try:
        await neo.call_tool("browser_act", {"kind": "click", "target": "Aceptar y cerrar"})
        await asyncio.sleep(1)
    except Exception:
        pass

    # ── Step 1: Set destination via React-compatible JS ──
    print(f"  [eDreams] Setting destination: {destination}")
    await eval_js(neo, f"""
    (() => {{
        const inp = document.querySelector('input[placeholder*="destino"]');
        if (!inp) return 'input not found';
        inp.focus(); inp.click();
        const nativeSet = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        nativeSet.call(inp, '{destination}');
        inp.dispatchEvent(new Event('input', {{ bubbles: true }}));
        inp.dispatchEvent(new Event('change', {{ bubbles: true }}));
        return 'ok';
    }})()
    """)
    await asyncio.sleep(2)

    # Select first autocomplete suggestion
    await eval_js(neo, """
    (() => {
        const span = document.querySelector('.odf-dropdown-highlight');
        if (!span) return 'no suggestions';
        let el = span;
        while (el && el.tagName !== 'LI') el = el.parentElement;
        if (el) el.click();
        return el?.innerText?.substring(0, 40) || 'clicked';
    })()
    """)
    await asyncio.sleep(1)

    # ── Step 2: Set dates via calendar ──
    print(f"  [eDreams] Setting dates: {checkin} → {checkout}")

    # Open check-in calendar
    await eval_js(neo, """
    (() => {
        const inp = document.querySelector('input[placeholder*="llegada"]');
        if (inp) { inp.focus(); inp.click(); }
        return inp ? 'opened' : 'not found';
    })()
    """)
    await asyncio.sleep(2)

    # Parse dates
    ci_day = int(checkin.split("-")[2])
    co_day = int(checkout.split("-")[2])

    # Navigate calendar to correct month if needed (check current month title)
    cal_title = await eval_js(neo, """
    (() => {
        const title = document.querySelector('.odf-calendar-title');
        return title?.innerText || '';
    })()
    """)

    # Click check-in day
    await eval_js(neo, f"""
    (() => {{
        const days = document.querySelectorAll('.odf-calendar-day:not(.disabled)');
        for (const d of days) {{
            if (d.innerText?.trim() === '{ci_day}') {{
                d.click();
                return 'clicked ' + d.innerText;
            }}
        }}
        return 'day {ci_day} not found';
    }})()
    """)
    await asyncio.sleep(1)

    # Click check-out day
    await eval_js(neo, f"""
    (() => {{
        const days = document.querySelectorAll('.odf-calendar-day:not(.disabled)');
        for (const d of days) {{
            if (d.innerText?.trim() === '{co_day}') {{
                d.click();
                return 'clicked ' + d.innerText;
            }}
        }}
        return 'day {co_day} not found';
    }})()
    """)
    await asyncio.sleep(1)

    # ── Step 3: Click search ──
    print(f"  [eDreams] Searching...")
    await neo.call_tool("browser_act", {"kind": "click", "target": "Buscar"})
    await asyncio.sleep(8)

    # Dismiss login/Prime popups
    for dismiss in ["Rechazar descuentos", "Cerrar", "No, gracias"]:
        try:
            await neo.call_tool("browser_act", {"kind": "click", "target": dismiss})
            await asyncio.sleep(1)
        except Exception:
            pass

    # ── Step 4: Activate SPA filter ──
    if spa_filter:
        print(f"  [eDreams] Activating SPA filter...")
        await eval_js(neo, """
        (() => {
            const cb = document.querySelector('input[value="FACILITY-SPA_SAUNA"]');
            if (cb && !cb.checked) {
                const label = cb.closest('label') || cb.parentElement;
                if (label) label.click();
                else cb.click();
            }
            return cb ? 'filtered' : 'no spa filter';
        })()
        """)
        await asyncio.sleep(5)

    # ── Step 5: Extract hotels from see output ──
    print(f"  [eDreams] Extracting results...")

    # Scroll down to load more
    await neo.call_tool("browser_act", {"kind": "scroll", "direction": "down"})
    await asyncio.sleep(2)

    page = await neo.observe("see")

    # Parse hotels from content section
    hotels = []
    content_start = page.find("Content:")
    if content_start < 0:
        content_start = 0
    content = page[content_start:]

    # Pattern: hotel name followed by location, then price
    # Example: "Daniya\n  Alicante\n  Vista Hermosa, Alicante · 3.97 km... 241 €"
    # Use regex on the full content
    lines = content.split('\n')
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        # Skip filter/noise lines
        if not line or line.startswith('#') or line.startswith('[') or len(line) < 3:
            i += 1
            continue

        # Look for price in nearby lines
        context = '\n'.join(lines[i:i+8])
        price_match = re.search(r'(\d{2,4})\s*€', context)
        km_match = re.search(r'(\d+[.,]?\d*)\s*km', context)

        # Check if this looks like a hotel name (no common noise words)
        noise = ['filtrar', 'ordenar', 'buscar', 'ofertas', 'instalaciones', 'presupuesto',
                 'tipo', 'política', 'barrio', 'categoría', 'distancia', 'marcas',
                 'confianza', 'millones', 'prime', 'desbloquear', 'listo', 'ahorra',
                 'explora', 'cómo', 'resultados', 'más información', 'mejor opción',
                 'criterios', 'configurar', 'valoración', 'según']

        if (price_match and km_match and
            len(line) > 3 and len(line) < 60 and
            not any(n in line.lower() for n in noise)):

            # Check next line for location context
            next_line = lines[i+1].strip() if i+1 < len(lines) else ""
            location = ""
            if km_match:
                loc_line = [l for l in lines[i:i+5] if 'km' in l]
                location = loc_line[0].strip() if loc_line else ""

            rating_match = re.search(r'(?:Bien|Muy bien|Fantástico|Excelente)\s*(\d)', context)

            hotel = {
                "name": line,
                "price": price_match.group(0),
                "location": location[:60],
                "rating": rating_match.group(0) if rating_match else "",
            }

            # Avoid duplicates
            if not any(h["name"] == hotel["name"] for h in hotels):
                hotels.append(hotel)

        i += 1

    # If regex parsing didn't work well, fallback: extract names from known patterns
    if len(hotels) < 2:
        print(f"  [eDreams] Regex found {len(hotels)}, trying JS fallback...")
        js_hotels = await eval_js(neo, """
        (() => {
            const page = document.body.innerText;
            const blocks = page.split(/\\n{2,}/);
            const hotels = [];
            for (const block of blocks) {
                const hasPrice = /\\d{2,4}\\s*€/.test(block);
                const hasKm = /\\d+[.,]?\\d*\\s*km/.test(block);
                if (hasPrice && hasKm && block.length > 30 && block.length < 500) {
                    const lines = block.trim().split('\\n');
                    const name = lines[0]?.trim();
                    const priceMatch = block.match(/(\\d{2,4})\\s*€/);
                    if (name && name.length > 3 && name.length < 60) {
                        hotels.push({
                            name,
                            price: priceMatch ? priceMatch[0] : '',
                            text: block.substring(0, 150),
                        });
                    }
                }
            }
            // Dedup
            const seen = new Set();
            return JSON.stringify(hotels.filter(h => {
                if (seen.has(h.name)) return false;
                seen.add(h.name);
                return true;
            }));
        })()
        """)
        if isinstance(js_hotels, list) and len(js_hotels) > len(hotels):
            hotels = js_hotels

    print(f"  [eDreams] Found {len(hotels)} hotels")
    for h in hotels:
        print(f"    {h.get('name','?'):<35} {h.get('price','?'):<8} {h.get('location','')[:30]}")

    return hotels
