#!/usr/bin/env python3
"""
Hotel Agent — search hotels with SPA, negotiate prices, monitor email.

Uses neobrowser-rs for web search/booking sites + Google API for Gmail.

Usage:
    python3 hotel_agent.py search --destination "Malaga" --checkin 2026-03-20 --checkout 2026-03-22
    python3 hotel_agent.py contact              # contact top hotels from last search
    python3 hotel_agent.py check-mail           # check Gmail for responses
    python3 hotel_agent.py negotiate            # auto-reply to hotel offers
    python3 hotel_agent.py status               # show negotiation status
"""

import argparse
import asyncio
import base64
import json
import os
import re
import sys
import time
from datetime import datetime, timedelta
from email.mime.text import MIMEText
from pathlib import Path
from urllib.parse import urlparse, quote_plus

sys.path.insert(0, "/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/ai-chat")
from aichat.neo_client import NeoClient

# Google API
from google.oauth2.credentials import Credentials
from google_auth_oauthlib.flow import InstalledAppFlow
from google.auth.transport.requests import Request
from googleapiclient.discovery import build
import google.auth

# ── Config ────────────────────────────────────────────────────────────

STATE_DIR = Path.home() / ".neobrowser" / "negotiations"
STATE_DIR.mkdir(parents=True, exist_ok=True)

GMAIL_SCOPES = ["https://www.googleapis.com/auth/gmail.modify"]
USER_EMAIL = "perezpinazo.daniel@gmail.com"
USER_NAME = "Daniel Pérez"

# Search parameters
BOOKING_URL = "https://www.booking.com"


# ── Gmail helpers ─────────────────────────────────────────────────────

def get_gmail_service():
    """Get authenticated Gmail API service using ADC."""
    creds, _ = google.auth.default(scopes=GMAIL_SCOPES)
    return build("gmail", "v1", credentials=creds)


def search_emails(service, query: str, max_results: int = 10) -> list[dict]:
    """Search Gmail for messages matching query."""
    results = service.users().messages().list(
        userId="me", q=query, maxResults=max_results
    ).execute()

    messages = []
    for msg_ref in results.get("messages", []):
        msg = service.users().messages().get(
            userId="me", id=msg_ref["id"], format="full"
        ).execute()

        headers = {h["name"]: h["value"] for h in msg["payload"]["headers"]}
        body = _extract_body(msg["payload"])

        messages.append({
            "id": msg["id"],
            "from": headers.get("From", ""),
            "to": headers.get("To", ""),
            "subject": headers.get("Subject", ""),
            "date": headers.get("Date", ""),
            "snippet": msg.get("snippet", ""),
            "body": body[:2000],
        })

    return messages


def _extract_body(payload: dict) -> str:
    """Extract text body from Gmail message payload."""
    if payload.get("body", {}).get("data"):
        return base64.urlsafe_b64decode(payload["body"]["data"]).decode(errors="replace")

    for part in payload.get("parts", []):
        if part["mimeType"] == "text/plain" and part.get("body", {}).get("data"):
            return base64.urlsafe_b64decode(part["body"]["data"]).decode(errors="replace")
        if part.get("parts"):
            result = _extract_body(part)
            if result:
                return result

    return ""


def send_email(service, to: str, subject: str, body: str):
    """Send an email via Gmail API."""
    message = MIMEText(body)
    message["to"] = to
    message["from"] = USER_EMAIL
    message["subject"] = subject

    raw = base64.urlsafe_b64encode(message.as_bytes()).decode()
    service.users().messages().send(
        userId="me", body={"raw": raw}
    ).execute()
    print(f"  [+] Email sent to {to}")


# ── State management ─────────────────────────────────────────────────

def load_state() -> dict:
    """Load negotiation state."""
    state_file = STATE_DIR / "state.json"
    if state_file.exists():
        return json.loads(state_file.read_text())
    return {
        "search": None,
        "hotels": [],
        "negotiations": {},
        "updated": None,
    }


def save_state(state: dict):
    """Save negotiation state."""
    state["updated"] = datetime.now().isoformat()
    state_file = STATE_DIR / "state.json"
    state_file.write_text(json.dumps(state, indent=2, ensure_ascii=False))


# ── Search ────────────────────────────────────────────────────────────

JS_EXTRACT_HOTELS = """
(() => {
    const cards = document.querySelectorAll('[data-testid="property-card"]');
    if (!cards.length) {
        // Fallback: try common booking.com selectors
        const alt = document.querySelectorAll('.sr_property_block, .d20f4628d0, [data-testid="property-card-container"]');
        if (!alt.length) return JSON.stringify({hotels: [], count: 0, fallback: true});
    }
    const hotels = [...(cards.length ? cards : document.querySelectorAll('.sr_property_block, .d20f4628d0'))].slice(0, 15).map(card => {
        const title = card.querySelector('[data-testid="title"], .sr-hotel__name, h3, [class*="title"]');
        const price = card.querySelector('[data-testid="price-and-discounted-price"], .bui-price-display__value, [class*="price"], [data-testid="price"]');
        const rating = card.querySelector('[data-testid="review-score"], .bui-review-score__badge, [class*="review-score"]');
        const link = card.querySelector('a[href]');
        const location = card.querySelector('[data-testid="address"], [class*="distance"], [class*="location"]');
        const img = card.querySelector('img[src]');

        return {
            name: title?.innerText?.trim() || '',
            price: price?.innerText?.trim() || '',
            rating: rating?.innerText?.trim() || '',
            url: link?.href || '',
            location: location?.innerText?.trim() || '',
            image: img?.src || '',
        };
    }).filter(h => h.name);
    return JSON.stringify({hotels, count: hotels.length});
})()
"""

JS_CHECK_SPA = """
((hotelName) => {
    const text = document.body.innerText.toLowerCase();
    const spaKeywords = ['spa', 'wellness', 'sauna', 'jacuzzi', 'hammam', 'masaje', 'massage',
                         'thermal', 'termal', 'hidromasaje', 'hydromassage', 'balneario',
                         'tratamiento', 'treatment', 'relax', 'piscina climatizada', 'heated pool'];
    const found = spaKeywords.filter(k => text.includes(k));
    return JSON.stringify({has_spa: found.length > 0, keywords: found, name: hotelName});
})
"""


async def eval_js(neo: NeoClient, js: str) -> dict:
    """Eval JS and parse the result JSON."""
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


async def cmd_search(args):
    """Search booking.com for hotels with SPA."""
    destination = args.destination
    checkin = args.checkin
    checkout = args.checkout

    # Build booking.com search URL with SPA filter
    # nflt=hotelfacility%3D54 = Spa/wellness filter on booking.com
    search_url = (
        f"{BOOKING_URL}/searchresults.html?"
        f"ss={quote_plus(destination)}"
        f"&checkin={checkin}"
        f"&checkout={checkout}"
        f"&nflt=hotelfacility%3D54"
        f"&order=popularity"
    )

    neo = NeoClient()
    try:
        print(f"[*] Starting browser...")
        await neo.start()

        print(f"[*] Searching: {destination}, {checkin} → {checkout}, SPA filter")
        await neo.call_tool("browser_open", {"url": search_url, "mode": "chrome"})
        await asyncio.sleep(4)  # booking.com is slow

        # Accept cookies if present
        try:
            await neo.call_tool("browser_act", {"kind": "click", "target": "Accept"})
            await asyncio.sleep(1)
        except Exception:
            pass

        # Extract hotel list
        print("[*] Extracting hotels...")
        hotels_data = await eval_js(neo, JS_EXTRACT_HOTELS)
        hotels = hotels_data.get("hotels", [])

        if not hotels:
            print("[!] No hotels found. Trying see mode fallback...")
            page_text = await neo.observe("see")
            print(page_text[:500])
            return

        print(f"[+] Found {len(hotels)} hotels")

        # Check each hotel for SPA details
        spa_hotels = []
        for i, hotel in enumerate(hotels[:10]):
            if not hotel.get("url"):
                continue

            print(f"  [{i+1}] {hotel['name']} — {hotel['price']}")

            # Visit hotel page to verify SPA
            try:
                await neo.call_tool("browser_open", {"url": hotel["url"], "mode": "chrome"})
                await asyncio.sleep(3)
                spa_info = await eval_js(neo, f"({JS_CHECK_SPA})('{hotel['name'].replace(chr(39), '')}')")

                if spa_info.get("has_spa"):
                    hotel["spa_keywords"] = spa_info["keywords"]
                    spa_hotels.append(hotel)
                    print(f"      ✓ SPA confirmed: {', '.join(spa_info['keywords'][:3])}")
                else:
                    print(f"      ✗ No SPA keywords found")

                # Extract contact info
                contact_js = """
                (() => {
                    const text = document.body.innerText;
                    const emailMatch = text.match(/[\\w.-]+@[\\w.-]+\\.[a-z]{2,}/i);
                    const phoneMatch = text.match(/[+]?[\\d\\s()-]{8,}/);
                    const website = document.querySelector('a[href*="hotel"][href*=".com"], a[href*="hotel"][href*=".es"]');
                    return JSON.stringify({
                        email: emailMatch ? emailMatch[0] : null,
                        phone: phoneMatch ? phoneMatch[0].trim() : null,
                        website: website ? website.href : null,
                    });
                })()
                """
                contact = await eval_js(neo, contact_js)
                hotel["contact"] = contact

            except Exception as e:
                print(f"      ! Error checking: {e}")

        # Save state
        state = load_state()
        state["search"] = {
            "destination": destination,
            "checkin": checkin,
            "checkout": checkout,
            "timestamp": datetime.now().isoformat(),
            "url": search_url,
        }
        state["hotels"] = spa_hotels
        save_state(state)

        # Summary
        print(f"\n{'='*60}")
        print(f"  Destination:  {destination}")
        print(f"  Dates:        {checkin} → {checkout}")
        print(f"  Hotels found: {len(hotels)} total, {len(spa_hotels)} with SPA")
        print(f"  State saved:  {STATE_DIR / 'state.json'}")
        print(f"{'='*60}")

        for i, h in enumerate(spa_hotels):
            print(f"\n  {i+1}. {h['name']}")
            print(f"     Price: {h['price']}")
            print(f"     Rating: {h.get('rating', 'N/A')}")
            print(f"     SPA: {', '.join(h.get('spa_keywords', []))}")
            if h.get("contact", {}).get("email"):
                print(f"     Email: {h['contact']['email']}")

    finally:
        await neo.stop()


# ── Contact hotels ────────────────────────────────────────────────────

CONTACT_TEMPLATE = """Estimado equipo del {hotel_name},

Me llamo {user_name} y estoy interesado en reservar una habitación doble con acceso al SPA para las fechas {checkin} - {checkout}.

He visto su hotel en Booking.com y el precio publicado es {published_price}. ¿Ofrecen algún precio especial para reserva directa? Estaría interesado en conocer:

1. Precio directo con acceso SPA incluido
2. Posibles paquetes o promociones disponibles
3. Política de cancelación

Quedo a la espera de su respuesta.

Un saludo,
{user_name}
{user_email}"""


async def cmd_contact(args):
    """Contact top hotels from search results."""
    state = load_state()
    hotels = state.get("hotels", [])

    if not hotels:
        print("[!] No hotels in state. Run 'search' first.")
        return

    search = state.get("search", {})
    service = get_gmail_service()

    contacted = 0
    for hotel in hotels:
        hotel_id = hotel["name"].lower().replace(" ", "_")[:30]

        # Skip already contacted
        if hotel_id in state.get("negotiations", {}):
            print(f"  [skip] {hotel['name']} — already contacted")
            continue

        # Need email to contact
        email = hotel.get("contact", {}).get("email")
        if not email:
            # Try to find email on hotel website
            website = hotel.get("contact", {}).get("website") or hotel.get("url", "")
            if website:
                print(f"  [*] Searching email for {hotel['name']}...")
                email = await _find_hotel_email(hotel["name"], website)

        if not email:
            print(f"  [!] No email found for {hotel['name']}, skipping")
            continue

        # Compose and send
        body = CONTACT_TEMPLATE.format(
            hotel_name=hotel["name"],
            user_name=USER_NAME,
            checkin=search.get("checkin", ""),
            checkout=search.get("checkout", ""),
            published_price=hotel.get("price", "N/A"),
            user_email=USER_EMAIL,
        )
        subject = f"Consulta reserva directa {search.get('checkin', '')} — {hotel['name']}"

        print(f"  [*] Contacting {hotel['name']} ({email})...")
        send_email(service, email, subject, body)

        state["negotiations"][hotel_id] = {
            "hotel": hotel["name"],
            "email": email,
            "status": "contacted",
            "published_price": hotel.get("price", ""),
            "contacted_at": datetime.now().isoformat(),
            "messages": [{"direction": "out", "date": datetime.now().isoformat(), "subject": subject}],
        }
        contacted += 1

    save_state(state)
    print(f"\n[+] Contacted {contacted} hotels. Run 'check-mail' to monitor responses.")


async def _find_hotel_email(name: str, url: str) -> str | None:
    """Try to find hotel email by visiting their website."""
    neo = NeoClient()
    try:
        await neo.start()
        await neo.call_tool("browser_open", {"url": url, "mode": "chrome"})
        await asyncio.sleep(3)

        # Look for contact/email on the page
        js = """
        (() => {
            const text = document.body.innerText + ' ' +
                [...document.querySelectorAll('a[href^="mailto:"]')].map(a => a.href).join(' ');
            const emails = text.match(/[\\w.-]+@[\\w.-]+\\.[a-z]{2,}/gi) || [];
            // Filter out generic emails
            const dominated = emails.filter(e =>
                !e.includes('example') && !e.includes('booking.com') && !e.includes('google')
            );
            return JSON.stringify({emails: dominated});
        })()
        """
        result = await eval_js(neo, js)
        emails = result.get("emails", [])

        # Prefer info@ or reservas@ or booking@
        for preferred in ["reserv", "booking", "info", "hotel", "contact"]:
            for email in emails:
                if preferred in email.lower():
                    return email

        return emails[0] if emails else None
    except Exception:
        return None
    finally:
        await neo.stop()


# ── Check mail ────────────────────────────────────────────────────────

async def cmd_check_mail(args):
    """Check Gmail for hotel responses."""
    state = load_state()
    negotiations = state.get("negotiations", {})

    if not negotiations:
        print("[!] No active negotiations. Run 'contact' first.")
        return

    service = get_gmail_service()

    # Search for responses from hotel emails
    hotel_emails = [n["email"] for n in negotiations.values() if n.get("email")]
    if not hotel_emails:
        print("[!] No hotel emails on record.")
        return

    query = " OR ".join(f"from:{email}" for email in hotel_emails)
    query += " newer_than:7d"

    print(f"[*] Checking Gmail for responses...")
    messages = search_emails(service, query)

    new_responses = 0
    for msg in messages:
        from_email = msg["from"]
        # Match to negotiation
        for neg_id, neg in negotiations.items():
            if neg["email"] in from_email:
                # Check if we already saw this message
                seen_ids = [m.get("id") for m in neg.get("messages", [])]
                if msg["id"] in seen_ids:
                    continue

                new_responses += 1
                neg["status"] = "responded"
                neg["messages"].append({
                    "direction": "in",
                    "id": msg["id"],
                    "date": msg["date"],
                    "subject": msg["subject"],
                    "snippet": msg["snippet"],
                    "body": msg["body"][:1000],
                })

                print(f"\n  [NEW] Response from {neg['hotel']}:")
                print(f"    Subject: {msg['subject']}")
                print(f"    Preview: {msg['snippet'][:150]}")

                # Try to extract price from response
                price_match = re.search(
                    r'(\d+[.,]?\d*)\s*€|€\s*(\d+[.,]?\d*)|(\d+[.,]?\d*)\s*eur',
                    msg["body"], re.IGNORECASE
                )
                if price_match:
                    offered = price_match.group(1) or price_match.group(2) or price_match.group(3)
                    neg["offered_price"] = f"{offered}€"
                    print(f"    Price detected: {offered}€")

                break

    save_state(state)

    if new_responses:
        print(f"\n[+] {new_responses} new response(s). Run 'negotiate' to auto-reply.")
    else:
        print("[*] No new responses yet.")


# ── Negotiate ─────────────────────────────────────────────────────────

COUNTER_TEMPLATE = """Estimado equipo del {hotel_name},

Muchas gracias por su respuesta y la oferta de {offered_price}.

He comparado con otros hoteles de la zona que ofrecen SPA incluido, y los precios directos rondan los {target_price}. ¿Sería posible ajustar el precio? Estaría dispuesto a confirmar la reserva hoy mismo si podemos llegar a un acuerdo.

Quedo atento a su respuesta.

Un saludo,
{user_name}"""

ACCEPT_TEMPLATE = """Estimado equipo del {hotel_name},

Acepto la oferta de {offered_price}. Por favor, envíenme los detalles para confirmar la reserva para las fechas {checkin} - {checkout}.

Muchas gracias,
{user_name}
{user_email}"""


async def cmd_negotiate(args):
    """Auto-reply to hotel offers based on price analysis."""
    state = load_state()
    negotiations = state.get("negotiations", {})
    search = state.get("search", {})

    responded = {k: v for k, v in negotiations.items() if v.get("status") == "responded"}

    if not responded:
        print("[!] No responses to negotiate. Run 'check-mail' first.")
        return

    service = get_gmail_service()

    for neg_id, neg in responded.items():
        print(f"\n[*] {neg['hotel']}:")
        print(f"    Published: {neg.get('published_price', 'N/A')}")
        print(f"    Offered:   {neg.get('offered_price', 'unknown')}")

        offered = neg.get("offered_price", "")
        published = neg.get("published_price", "")

        # Parse prices for comparison
        offered_num = _parse_price(offered)
        published_num = _parse_price(published)

        if offered_num and published_num:
            discount = ((published_num - offered_num) / published_num) * 100

            if discount >= 15:
                # Good deal — accept
                print(f"    Discount: {discount:.0f}% — ACCEPTING")
                body = ACCEPT_TEMPLATE.format(
                    hotel_name=neg["hotel"],
                    offered_price=offered,
                    checkin=search.get("checkin", ""),
                    checkout=search.get("checkout", ""),
                    user_name=USER_NAME,
                    user_email=USER_EMAIL,
                )
                subject = f"Re: Confirmación reserva — {neg['hotel']}"
                send_email(service, neg["email"], subject, body)
                neg["status"] = "accepted"

            elif discount >= 5:
                # Decent but try for more
                target = int(published_num * 0.80)
                print(f"    Discount: {discount:.0f}% — COUNTER-OFFERING {target}€")
                body = COUNTER_TEMPLATE.format(
                    hotel_name=neg["hotel"],
                    offered_price=offered,
                    target_price=f"{target}€",
                    user_name=USER_NAME,
                )
                subject = f"Re: Consulta reserva — {neg['hotel']}"
                send_email(service, neg["email"], subject, body)
                neg["status"] = "negotiating"
                neg["messages"].append({
                    "direction": "out",
                    "date": datetime.now().isoformat(),
                    "subject": subject,
                    "type": "counter_offer",
                    "target_price": f"{target}€",
                })

            else:
                # Not enough discount — counter
                target = int(published_num * 0.75)
                print(f"    Discount: {discount:.0f}% — COUNTER-OFFERING {target}€ (aggressive)")
                body = COUNTER_TEMPLATE.format(
                    hotel_name=neg["hotel"],
                    offered_price=offered,
                    target_price=f"{target}€",
                    user_name=USER_NAME,
                )
                subject = f"Re: Consulta reserva — {neg['hotel']}"
                send_email(service, neg["email"], subject, body)
                neg["status"] = "negotiating"
        else:
            print(f"    Cannot parse prices — manual review needed")
            neg["status"] = "needs_review"

    save_state(state)
    print(f"\n[+] Negotiation round complete. Run 'check-mail' again later.")


def _parse_price(price_str: str) -> float | None:
    """Extract numeric price from string like '120€', '€ 85.50', etc."""
    if not price_str:
        return None
    match = re.search(r'(\d+[.,]?\d*)', price_str.replace(".", "").replace(",", "."))
    if match:
        try:
            return float(match.group(1))
        except ValueError:
            return None
    return None


# ── Status ────────────────────────────────────────────────────────────

async def cmd_status(args):
    """Show negotiation status."""
    state = load_state()

    search = state.get("search")
    if search:
        print(f"Search: {search['destination']}, {search['checkin']} → {search['checkout']}")
        print(f"Hotels with SPA: {len(state.get('hotels', []))}")
    else:
        print("No search performed yet.")
        return

    negotiations = state.get("negotiations", {})
    if not negotiations:
        print("No negotiations started.")
        return

    print(f"\nNegotiations ({len(negotiations)}):")
    for neg_id, neg in negotiations.items():
        status_icon = {
            "contacted": "📤",
            "responded": "📩",
            "negotiating": "🔄",
            "accepted": "✅",
            "rejected": "❌",
            "needs_review": "⚠️",
        }.get(neg["status"], "?")

        print(f"  {status_icon} {neg['hotel']}")
        print(f"     Status: {neg['status']}")
        print(f"     Published: {neg.get('published_price', 'N/A')}")
        if neg.get("offered_price"):
            print(f"     Offered: {neg['offered_price']}")
        print(f"     Messages: {len(neg.get('messages', []))}")


# ── CLI ───────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Hotel Agent — search, contact, negotiate")
    sub = parser.add_subparsers(dest="command")

    p_search = sub.add_parser("search", help="Search hotels with SPA")
    p_search.add_argument("--destination", required=True, help="City/region")
    p_search.add_argument("--checkin", required=True, help="Check-in date (YYYY-MM-DD)")
    p_search.add_argument("--checkout", required=True, help="Check-out date (YYYY-MM-DD)")

    sub.add_parser("contact", help="Contact top hotels from last search")
    sub.add_parser("check-mail", help="Check Gmail for hotel responses")
    sub.add_parser("negotiate", help="Auto-reply to hotel offers")
    sub.add_parser("status", help="Show negotiation status")

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return

    cmd_map = {
        "search": cmd_search,
        "contact": cmd_contact,
        "check-mail": cmd_check_mail,
        "negotiate": cmd_negotiate,
        "status": cmd_status,
    }

    asyncio.run(cmd_map[args.command](args))


if __name__ == "__main__":
    main()
