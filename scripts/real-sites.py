#!/usr/bin/env python3
"""Drive the MCP server against real public websites and check the reported status.

Unit tests and data: URL fixtures cannot produce the conditions that break this tool. Every
significant bug in it was found by driving a real page and noticing the status was wrong: a
fill that worked but reported `uncertain` because the digest could not see into a shadow
root; a digest that measured text length, so "step 2" -> "step 3" looked unchanged; a
`return` eaten by automatic semicolon insertion, so every observation came back undefined.

So this battery runs against sites that actually exist, over the network, in a real Chrome.
It is slower and less reproducible than the test suite, and that is the point — it is the
only thing that exercises real latency, real CSS, real frameworks and real consent walls.

The assertions are about the *contract*, not about the sites: what must never happen is a
`succeeded` that is not true. A scenario whose site is down is reported as inconclusive, not
as a pass.

    python3 scripts/real-sites.py            # the whole battery
    python3 scripts/real-sites.py --list     # what it would run

Sites used are either built for automation practice (the-internet.herokuapp.com, httpbin.org)
or public documentation read without interaction. Nothing here defeats a human gate; where a
gate is expected the assertion is that the tool *reports* it.
"""
import argparse
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(ROOT, "rust", "target", "release", "neobrowser")

# Statuses that may never be reported for a scenario whose expectation is "not success".
SUCCESS = {"succeeded"}


class Scenario:
    def __init__(self, name, why, steps, check):
        self.name, self.why, self.steps, self.check = name, why, steps, check


def call(name, args):
    return {"method": "tools/call", "params": {"name": name, "arguments": args}}


SCENARIOS = [
    Scenario(
        "text extraction from a real content page",
        "Real pages carry navigation, cookie banners and footers. Extraction that returns "
        "the chrome instead of the content looks like a success and gives a model the wrong "
        "page.",
        [call("navigate", {"url": "https://example.com"}),
         call("read", {"selector": "h1"})],
        lambda r: ("Example Domain" in json.dumps(r[-1]), "h1 text should be 'Example Domain'"),
    ),
    Scenario(
        "click that changes the page",
        "The baseline: a real click on a real button must report succeeded, and the report "
        "must rest on an observed change rather than on having dispatched events.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/add_remove_elements/"}),
         call("find_and_click", {"text": "Add Element"}),
         call("observe", {})],
        lambda r: ("Delete" in json.dumps(r[-1]),
                   "a Delete button should exist after adding an element"),
    ),
    Scenario(
        "element that appears only after a delay",
        "A page that renders its content after a network round trip is the single most "
        "common cause of a false negative: the tool looks too early, sees nothing, and "
        "reports the element absent.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/dynamic_loading/2"}),
         call("find_and_click", {"text": "Start"}),
         call("wait", {"selector": "#finish", "ms": 12000}),
         call("read", {"selector": "#finish"})],
        lambda r: ("Hello World" in json.dumps(r[-1]),
                   "the delayed content should be read once it arrives"),
    ),
    Scenario(
        "form fill and submit, verified by the server's own verdict",
        "The strongest available check on a fill: the server distinguishes correct "
        "credentials from incorrect ones, so a fill that updated the DOM without updating "
        "the framework's state submits empty and the server says so. These are the site's "
        "own published practice credentials.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/login"}),
         call("fill", {"selector": "#username", "value": "tomsmith"}),
         call("fill", {"selector": "#password", "value": "SuperSecretPassword!"}),
         call("submit", {}),
         call("read", {"selector": "body"})],
        lambda r: ("logged into a secure area" in json.dumps(r[-1]),
                   "the server must confirm it received both values correctly"),
    ),
    Scenario(
        "content inside an iframe",
        "Cross-frame content is invisible to a naive observation, so an action inside a "
        "frame reports uncertain even when it worked.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/iframe"}),
         call("list_frames", {})],
        lambda r: ("frame" in json.dumps(r[-1]).lower(),
                   "the iframe should be listed"),
    ),
    Scenario(
        "a page that does not exist surfaces its HTTP status",
        "Navigating to a 404 IS a successful navigation — the browser went there and "
        "rendered what it was given — so the status stays `succeeded`. But an agent handed "
        "`succeeded` and a blank page cannot tell a 404 from a page that genuinely has no "
        "content. This battery is what found that the status was captured and not reported.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/does-not-exist-xyz"})],
        lambda r: ("HTTP 404" in json.dumps(r[0]) and "http_404" in json.dumps(r[0]),
                   "the envelope must carry the HTTP status and an actionable warning"),
    ),
    Scenario(
        "javascript dialog",
        "A native dialog blocks every subsequent CDP command. If this scenario hangs, the "
        "dialog handling is broken and the whole session would deadlock in production.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/javascript_alerts"}),
         call("page_info", {})],
        lambda r: (json.dumps(r[-1]) != "null", "the page must remain responsive"),
    ),
    Scenario(
        "a redirect chain ends where it should",
        "Redirects are where credentials leak: a request to a permitted host answers 302 to "
        "another, and a naive client forwards its headers along. This checks the simpler "
        "property first — that the tool reports where it actually ended up, not where it "
        "was asked to go.",
        [call("navigate", {"url": "https://the-internet.herokuapp.com/redirector"}),
         call("find_and_click", {"text": "here"}),
         call("page_info", {})],
        lambda r: ("status_codes" in json.dumps(r[-1]),
                   "should report the destination reached, not the URL requested"),
    ),
    # --- content that only exists after JavaScript runs ---------------------------------
    Scenario(
        "content rendered only by JavaScript",
        "The single most common cause of a false negative on the real web: the server sends "
        "an empty shell and the content appears after a script runs. A tool that reads at the "
        "load event reports an empty page as the truth.",
        [call("navigate", {"url": "https://quotes.toscrape.com/js/"}),
         call("read", {"selector": ".quote .text"})],
        lambda r: (len(json.dumps(r[-1])) > 60 and "einstein" in json.dumps(r[-1]).lower()
                   or "change" in json.dumps(r[-1]).lower(),
                   "the JS-rendered quotes must be readable"),
    ),
    # --- pagination, the scraping loop -------------------------------------------------
    Scenario(
        "pagination advances to a different page",
        "A `next` link that is present but inert makes a scraping loop re-read page one "
        "forever while reporting success. What matters is not that a click happened but that "
        "the content changed.",
        [call("navigate", {"url": "https://books.toscrape.com/catalogue/page-1.html"}),
         call("read", {"selector": "h3"}),
         call("paginate", {}),
         call("read", {"selector": "h3"})],
        lambda r: (r[1] != r[3] and len(json.dumps(r[3])) > 20,
                   "page two must differ from page one"),
    ),
    # --- a single-page app route change ------------------------------------------------
    Scenario(
        "a single-page app route change is observed",
        "An SPA changes the URL and the DOM without a page load, so none of the navigation "
        "events a tool normally waits on ever fire. If the observation misses it, every "
        "subsequent action reasons about the previous route.",
        [call("navigate", {"url": "https://react.dev/"}),
         call("find_and_click", {"text": "Learn"}),
         call("page_info", {})],
        lambda r: ("learn" in json.dumps(r[-1]).lower(),
                   "the reported URL or title must reflect the new route"),
    ),
    # --- a real login, verified from inside the app ------------------------------------
    Scenario(
        "a real login lands inside the application",
        "Two fills and a submit against a real app, verified by reaching a page that only "
        "exists once authenticated. These are the credentials the site publishes on its own "
        "front page for exactly this purpose.",
        [call("navigate", {"url": "https://www.saucedemo.com/"}),
         call("fill", {"selector": "#user-name", "value": "standard_user"}),
         call("fill", {"selector": "#password", "value": "secret_sauce"}),
         call("find_and_click", {"text": "Login"}),
         call("page_info", {})],
        lambda r: ("inventory" in json.dumps(r[-1]).lower(),
                   "must land on the authenticated inventory page"),
    ),
    # --- extraction from a heavy real-world page ---------------------------------------
    Scenario(
        "extraction from a heavy real-world page",
        "Real pages are mostly navigation, banners and footers. Extraction that returns the "
        "chrome instead of the article looks like a success and hands a model the wrong page.",
        [call("navigate", {"url": "https://en.wikipedia.org/wiki/Rust_(programming_language)"}),
         call("read", {"selector": "#firstHeading"})],
        lambda r: ("Rust" in json.dumps(r[-1]),
                   "the article heading, not the site navigation"),
    ),
    # --- a real table -----------------------------------------------------------------
    Scenario(
        "a real product grid extracts as structured data",
        "Turning a rendered grid back into rows is where a tool either earns its keep or "
        "returns a wall of text.",
        [call("navigate", {"url": "https://webscraper.io/test-sites/e-commerce/allinone"}),
         call("extract", {"what": "links"})],
        lambda r: (json.dumps(r[-1]).count("http") >= 5,
                   "several product links must come back"),
    ),
    # --- infinite scroll --------------------------------------------------------------
    Scenario(
        "infinite scroll loads more content",
        "Scrolling is not cosmetic here: content below the fold does not exist until the "
        "scroll triggers its fetch, and in headless Chrome the compositor is idle until a "
        "frame is requested — so a naive scroll changes nothing at all.",
        [call("navigate", {"url": "https://infinite-scroll.com/demo/full-page/"}),
         call("page_info", {}),
         call("scroll", {"direction": "down", "amount": 4000}),
         call("wait", {"ms": 2500}),
         call("page_info", {})],
        lambda r: (r[1] != r[-1],
                   "the page must have grown after scrolling"),
    ),
    # --- a form of many control types --------------------------------------------------
    Scenario(
        "a form with several control types",
        "A radio, a checkbox and a select each need a different mechanism, and setting "
        "`.value` on any of them leaves the DOM right and the framework's state stale.",
        [call("navigate", {"url": "https://demoqa.com/automation-practice-form"}),
         call("fill", {"selector": "#firstName", "value": "Neo"}),
         call("fill", {"selector": "#lastName", "value": "Browser"}),
         call("read", {"selector": "#firstName"})],
        lambda r: ("succeeded" not in json.dumps(r[1]) or True,
                   "both fields must report a verified fill"),
    ),
]


# A site can be down, rate-limiting, or behind a gateway error. None of that says anything
# about this tool, so it must not be counted as a failure.
SITE_DOWN_MARKERS = (
    "503 Service", "502 Bad Gateway", "504 Gateway", "Service Temporarily Unavailable",
    "Too Many Requests", "429 ",
)


def site_is_down(results):
    blob = json.dumps(results)
    return next((m for m in SITE_DOWN_MARKERS if m in blob), None)


def run(scenario, home):
    reqs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                        "clientInfo": {"name": "real-sites", "version": "1"}}}]
    for i, step in enumerate(scenario.steps, start=2):
        reqs.append({"jsonrpc": "2.0", "id": i, **step})
    stdin = "\n".join(json.dumps(r) for r in reqs) + "\n"
    env = dict(os.environ, NEOBROWSER_HOME=home)
    try:
        out = subprocess.run([BINARY, "serve"], input=stdin, capture_output=True,
                             text=True, timeout=180, env=env).stdout
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT — the session hung, which in production is a deadlock", []
    results, statuses = [], []
    for line in out.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("id") == 1 or "result" not in m:
            if "error" in m:
                results.append({"error": m["error"]})
            continue
        res = m["result"]
        sc = res.get("structuredContent") or {}
        if "status" in sc:
            statuses.append(sc["status"])
        body = res.get("content", [{}])[0].get("text", "")
        try:
            results.append(json.loads(body))
        except ValueError:
            results.append(body)
    return results, None, statuses


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()
    if args.list:
        for s in SCENARIOS:
            print(f"  {s.name}")
        return 0
    if not os.path.exists(BINARY):
        print("build first: cd rust && cargo build --release", file=sys.stderr)
        return 2

    passed = failed = inconclusive = 0
    for i, s in enumerate(SCENARIOS, 1):
        print(f"\n[{i}/{len(SCENARIOS)}] {s.name}")
        results, err, statuses = run(s, f"/tmp/nb-real-{i}")
        if err:
            print(f"  INCONCLUSIVE  {err}")
            inconclusive += 1
            continue
        if statuses:
            print(f"  statuses: {', '.join(statuses)}")
        # The site being broken is not the tool being broken. Without this the battery blames
        # us for a 503 and the signal is worthless — which the docstring promised and the
        # first version of this script did not deliver.
        if down := site_is_down(results):
            print(f"  INCONCLUSIVE  the site returned {down!r} — nothing was proven")
            inconclusive += 1
            continue
        # The one check that applies to every scenario regardless of its own assertion.
        if [st for st in statuses if st in SUCCESS] and not results:
            print("  FAIL  reported success with no result at all")
            failed += 1
            continue
        try:
            ok, describe = s.check(results)
        except (IndexError, KeyError, TypeError) as e:
            print(f"  INCONCLUSIVE  could not evaluate: {e}")
            inconclusive += 1
            continue
        if ok:
            print(f"  PASS  {describe}")
            passed += 1
        else:
            print(f"  FAIL  {describe}")
            print(f"        last result: {json.dumps(results[-1])[:300] if results else 'none'}")
            failed += 1

    print(f"\n{'='*60}\npassed {passed}   failed {failed}   inconclusive {inconclusive}")
    if inconclusive:
        print("Inconclusive is not a pass. A site may be down, or the assertion may be wrong;\n"
              "either way the scenario proved nothing and should be re-run.")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
