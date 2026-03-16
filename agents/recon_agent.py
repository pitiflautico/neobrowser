#!/usr/bin/env python3
"""
recon_agent.py — Bounty hunting / security recon agent using neobrowser-rs.

Connects to neobrowser via NeoClient, crawls a target site, captures network
traffic, identifies API endpoints, and tests for common security issues
(IDOR, missing auth, info disclosure).

Usage: python3 recon_agent.py https://target.com
"""

import asyncio
import json
import sys
import time
from dataclasses import dataclass, field
from urllib.parse import urljoin, urlparse

sys.path.insert(0, "/Volumes/DiscoExterno2/mac_offload/Projects/meta-agente/lab/ai-chat")
from aichat.neo_client import NeoClient

# ── Paths to probe ──────────────────────────────────────────────────────────
COMMON_PATHS = [
    "/", "/login", "/signin", "/signup", "/register",
    "/api", "/api/v1", "/api/v2", "/graphql",
    "/admin", "/dashboard", "/profile", "/settings",
    "/health", "/status", "/debug", "/metrics",
    "/.env", "/config", "/swagger", "/api-docs", "/openapi.json",
    "/robots.txt", "/sitemap.xml", "/.well-known/security.txt",
]

IDOR_PATTERNS = [
    "/profile/{id}", "/user/{id}", "/account/{id}",
    "/api/user/{id}", "/api/v1/user/{id}", "/api/account/{id}",
    "/api/order/{id}", "/api/invoice/{id}",
]

DEBUG_SIGNALS = [
    "stack trace", "traceback", "exception", "debug", "internal server error",
    "x-powered-by", "server:", "x-debug", "x-request-id",
    "django", "laravel", "express", "flask", "rails",
]


@dataclass
class Finding:
    severity: str  # HIGH, MEDIUM, LOW, INFO
    category: str
    detail: str
    evidence: str = ""


@dataclass
class ReconState:
    target: str
    started: float = 0.0
    endpoints: list = field(default_factory=list)
    api_calls: list = field(default_factory=list)
    auth_endpoints: list = field(default_factory=list)
    graphql: list = field(default_factory=list)
    findings: list = field(default_factory=list)
    pages_visited: int = 0
    requests_captured: int = 0


async def crawl_paths(neo: NeoClient, base: str, state: ReconState):
    """Visit common paths, capture status codes and responses."""
    print(f"\n[CRAWL] Probing {len(COMMON_PATHS)} paths on {base}")
    for path in COMMON_PATHS:
        url = urljoin(base, path)
        try:
            resp = await neo.call_tool("browser_api", {
                "url": url, "method": "GET", "extract": "json",
            })
            text = resp if isinstance(resp, str) else json.dumps(resp)
            status = "200"  # browser_api returns content on success

            state.endpoints.append({"url": url, "status": status, "size": len(text)})
            state.pages_visited += 1

            # Check for info disclosure signals
            text_lower = text.lower()
            for signal in DEBUG_SIGNALS:
                if signal in text_lower:
                    state.findings.append(Finding(
                        severity="MEDIUM", category="info-disclosure",
                        detail=f"Debug signal '{signal}' found at {url}",
                        evidence=text[:200],
                    ))
            print(f"  [OK]  {path} ({len(text)} bytes)")
        except Exception as e:
            err = str(e)
            if "404" not in err and "403" not in err:
                print(f"  [ERR] {path}: {err[:80]}")
            else:
                print(f"  [---] {path}: blocked/missing")


async def analyze_traffic(neo: NeoClient, state: ReconState):
    """Read captured network traffic and classify endpoints."""
    print("\n[NETWORK] Reading captured traffic...")
    try:
        traffic = await neo.call_tool("browser_network", {"op": "read"})
        text = traffic if isinstance(traffic, str) else json.dumps(traffic)
        entries = []
        try:
            entries = json.loads(text) if isinstance(traffic, str) else traffic
            if isinstance(entries, dict):
                entries = entries.get("entries", entries.get("requests", [entries]))
        except (json.JSONDecodeError, TypeError):
            pass

        if not isinstance(entries, list):
            entries = []

        state.requests_captured = len(entries)
        print(f"  Captured {len(entries)} requests")

        for entry in entries:
            if not isinstance(entry, dict):
                continue
            url = entry.get("url", entry.get("request", {}).get("url", ""))
            method = entry.get("method", entry.get("request", {}).get("method", "GET"))
            url_lower = url.lower()

            if "/api/" in url_lower or "/graphql" in url_lower or "/rest/" in url_lower:
                state.api_calls.append({"url": url, "method": method})

            if "graphql" in url_lower:
                state.graphql.append(url)

            if any(k in url_lower for k in ["login", "auth", "token", "session", "oauth"]):
                state.auth_endpoints.append({"url": url, "method": method})

    except Exception as e:
        print(f"  [ERR] Failed to read traffic: {e}")


async def test_idor(neo: NeoClient, base: str, state: ReconState):
    """Test IDOR by accessing sequential resource IDs."""
    print("\n[IDOR] Testing sequential ID access...")
    test_ids = ["1", "2", "3", "100", "admin"]

    for pattern in IDOR_PATTERNS:
        responses = {}
        for rid in test_ids:
            url = urljoin(base, pattern.replace("{id}", rid))
            try:
                resp = await neo.call_tool("browser_api", {
                    "url": url, "method": "GET", "extract": "json",
                })
                text = resp if isinstance(resp, str) else json.dumps(resp)
                responses[rid] = len(text)
            except Exception:
                responses[rid] = -1

        # If multiple IDs return different-sized valid responses, potential IDOR
        valid = {k: v for k, v in responses.items() if v > 0}
        if len(valid) >= 2 and len(set(valid.values())) > 1:
            state.findings.append(Finding(
                severity="HIGH", category="idor",
                detail=f"Multiple valid responses for {pattern}",
                evidence=json.dumps(valid),
            ))
            print(f"  [!!] {pattern} — different responses for IDs: {valid}")
        elif valid:
            print(f"  [?]  {pattern} — {len(valid)} valid responses (same size)")
        else:
            print(f"  [--] {pattern} — no valid responses")


async def test_missing_auth(neo: NeoClient, state: ReconState):
    """Replay discovered API calls to check if auth is enforced."""
    print("\n[AUTH] Testing API calls without authentication...")
    if not state.api_calls:
        print("  No API calls discovered, skipping")
        return

    # Clear cookies to simulate unauthenticated request
    try:
        await neo.call_tool("browser_act", {
            "kind": "eval",
            "text": "document.cookie.split(';').forEach(c => { document.cookie = c.trim().split('=')[0] + '=;expires=Thu, 01 Jan 1970 00:00:00 GMT;path=/'; })",
        })
    except Exception:
        pass

    tested = set()
    for call in state.api_calls[:15]:  # Limit to 15 endpoints
        url = call["url"]
        if url in tested:
            continue
        tested.add(url)

        try:
            resp = await neo.call_tool("browser_api", {
                "url": url, "method": call.get("method", "GET"), "extract": "json",
            })
            text = resp if isinstance(resp, str) else json.dumps(resp)
            # If we get a non-error response without cookies, auth may be missing
            if len(text) > 50 and "unauthorized" not in text.lower() and "forbidden" not in text.lower():
                state.findings.append(Finding(
                    severity="HIGH", category="missing-auth",
                    detail=f"API responds without auth: {call['method']} {url}",
                    evidence=text[:300],
                ))
                print(f"  [!!] {call['method']} {url} — responds without auth ({len(text)} bytes)")
            else:
                print(f"  [OK] {call['method']} {url} — auth enforced")
        except Exception:
            print(f"  [OK] {call['method']} {url} — rejected")


async def export_results(neo: NeoClient, state: ReconState):
    """Export HAR and browser state to /tmp/."""
    domain = urlparse(state.target).hostname or "unknown"
    ts = int(time.time())
    prefix = f"/tmp/recon_{domain}_{ts}"

    # Export HAR
    try:
        har = await neo.call_tool("browser_network", {"op": "har"})
        har_path = f"{prefix}.har"
        with open(har_path, "w") as f:
            json.dump(har if isinstance(har, dict) else {"raw": har}, f, indent=2)
        print(f"\n[EXPORT] HAR saved to {har_path}")
    except Exception as e:
        print(f"\n[EXPORT] HAR export failed: {e}")

    # Export browser state
    try:
        browser_state = await neo.call_tool("browser_state", {"op": "export"})
        state_path = f"{prefix}_state.json"
        with open(state_path, "w") as f:
            json.dump(browser_state if isinstance(browser_state, dict) else {"raw": browser_state}, f, indent=2)
        print(f"[EXPORT] State saved to {state_path}")
    except Exception as e:
        print(f"[EXPORT] State export failed: {e}")

    # Export findings
    findings_path = f"{prefix}_findings.json"
    findings_data = [
        {"severity": f.severity, "category": f.category, "detail": f.detail, "evidence": f.evidence}
        for f in state.findings
    ]
    with open(findings_path, "w") as f:
        json.dump(findings_data, f, indent=2)
    print(f"[EXPORT] Findings saved to {findings_path}")


def print_report(state: ReconState):
    """Print structured findings to stdout."""
    elapsed = time.time() - state.started
    print("\n" + "=" * 70)
    print(f"RECON REPORT — {state.target}")
    print(f"Duration: {elapsed:.1f}s | Pages: {state.pages_visited} | "
          f"Requests captured: {state.requests_captured}")
    print("=" * 70)

    print(f"\nEndpoints discovered: {len(state.endpoints)}")
    print(f"API calls found:     {len(state.api_calls)}")
    print(f"Auth endpoints:      {len(state.auth_endpoints)}")
    print(f"GraphQL endpoints:   {len(state.graphql)}")

    if state.api_calls:
        print("\n── API Endpoints ──")
        for c in state.api_calls[:20]:
            print(f"  {c['method']:6s} {c['url']}")

    if state.auth_endpoints:
        print("\n── Auth Endpoints ──")
        for c in state.auth_endpoints[:10]:
            print(f"  {c['method']:6s} {c['url']}")

    if state.graphql:
        print("\n── GraphQL ──")
        for url in set(state.graphql):
            print(f"  {url}")

    if state.findings:
        by_sev = {"HIGH": [], "MEDIUM": [], "LOW": [], "INFO": []}
        for f in state.findings:
            by_sev.get(f.severity, by_sev["INFO"]).append(f)

        print(f"\n── Findings ({len(state.findings)}) ──")
        for sev in ("HIGH", "MEDIUM", "LOW", "INFO"):
            for f in by_sev[sev]:
                marker = {"HIGH": "!!", "MEDIUM": "!", "LOW": "~", "INFO": "."}[sev]
                print(f"  [{marker}] [{sev}] {f.category}: {f.detail}")
                if f.evidence:
                    print(f"       evidence: {f.evidence[:120]}")
    else:
        print("\n  No findings.")

    print("\n" + "=" * 70)


async def main(target: str):
    state = ReconState(target=target, started=time.time())
    neo = NeoClient()

    try:
        print(f"[INIT] Starting neobrowser for {target}...")
        await neo.start()

        # Enable tracing for stats
        await neo.call_tool("browser_trace", {"op": "start"})

        # Open target and start network capture
        await neo.call_tool("browser_network", {"op": "start"})
        await neo.call_tool("browser_open", {"url": target, "mode": "chrome"})
        print(f"[INIT] Target loaded, network capture active")

        # Phase 1: Crawl common paths
        await crawl_paths(neo, target, state)

        # Phase 2: Analyze captured traffic
        await analyze_traffic(neo, state)

        # Phase 3: Test IDOR
        await test_idor(neo, target, state)

        # Phase 4: Test missing auth on discovered APIs
        await test_missing_auth(neo, state)

        # Phase 5: Export artifacts
        await export_results(neo, state)

        # Print report
        print_report(state)

        # Print trace stats
        try:
            stats = await neo.call_tool("browser_trace", {"op": "stats"})
            print(f"\n[TRACE] {stats}")
        except Exception:
            pass

    except KeyboardInterrupt:
        print("\n[ABORT] Interrupted by user")
    except Exception as e:
        print(f"\n[FATAL] {e}")
        raise
    finally:
        await neo.stop()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python3 recon_agent.py https://target.com")
        sys.exit(1)
    asyncio.run(main(sys.argv[1]))
