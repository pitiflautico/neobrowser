#!/usr/bin/env python3
"""Minimal reproduction: CDP Input silently stops being delivered to a page.

    python3 scripts/repro-input-stall.py

Needs Chrome and `pip install websockets`. Uses no part of this crate — a bare CDP client, so
the result is about Chrome rather than about NeoBrowser.

## What it shows

Navigate to https://www.saucedemo.com/, fill the login form, click the submit button with
`Input.dispatchMouseEvent`. The login works and the page reaches /inventory.html. From that
moment on, every `Input.dispatchMouseEvent` and `Input.dispatchKeyEvent` on that target is
accepted without error and never delivered. The probe here installs a capturing `mousemove`
listener on `document` and counts what arrives: zero, at any coordinate.

Meanwhile the same target keeps working for everything else — `Runtime.evaluate`,
`DOM.getBoxModel`, `Page.captureScreenshot` — and a JavaScript `.click()` on the very button
that will not respond to a real click works immediately.

## Observed on

Chrome 151.0.7922.138, macOS (Darwin 25.4.0). Reproduces in `--headless=new` and with a
visible window, on a fresh profile, with no extensions or flags beyond those below.

## Ruled out

Each of these was tested and does not explain or cure it:

  - window focus and occlusion; `Page.bringToFront`; `Target.activateTarget`
  - an idle compositor: forcing frames with `Page.captureScreenshot` changes nothing
  - coordinate space: a grid sweep from (20,20) to (1800,900) delivers nothing anywhere, and
    `Page.getLayoutMetrics` agrees with `window.innerWidth/innerHeight`
  - `Emulation.setFocusEmulationEnabled`, with and without
  - `Emulation.setDeviceMetricsOverride`, setting and clearing it
  - `Input.setIgnoreInputEvents(false)`
  - a stuck mouse button or lost pointer capture: a standalone `mouseReleased`, and a
    `mouseMoved`+`mouseReleased` pair, do not restore it — nor does releasing with no pause
    between press and release
  - target or frame churn: `Target.getTargets` and `Page.getFrameTree` are identical before and
    after, same target id, one frame, no out-of-process iframe
  - `Page.reload`, `Page.navigate` to the same URL, and reconnecting the websocket to the same
    target
  - a page-side cause: the button's handler works when invoked from JavaScript

A *different tab in the same browser* has fully working input, including with this crate's
entire setup applied to it. That is the only known recovery, and it is what
`Browser::replace_active_tab` does.

## Why it matters here

The failure is silent by construction: Chrome accepts the command. A tool that reports "clicked"
on a dispatch would report success forever after this point. NeoBrowser reports `uncertain`,
probes with `page::input_is_alive` on the failure path, and replaces the tab — see
`docs/VERIFIED-ACTIONS.md` for why an honest `uncertain` is the requirement rather than a
courtesy.
"""
import asyncio
import json
import shutil
import subprocess
import time
import urllib.request

import websockets

CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PORT = 9503
PROFILE = "/tmp/nb-stuck"


def rpc_of(ws, n):
    async def rpc(method, params=None):
        n[0] += 1
        await ws.send(json.dumps({"id": n[0], "method": method, "params": params or {}}))
        while True:
            m = json.loads(await ws.recv())
            if m.get("id") == n[0]:
                return m
    return rpc


async def session(rpc):
    async def js(e):
        r = await rpc("Runtime.evaluate", {"expression": e, "returnByValue": True})
        return r.get("result", {}).get("result", {}).get("value")

    async def alive():
        await js("window.__p=0;document.addEventListener('mousemove',"
                 "function(){window.__p++},true);1")
        await rpc("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": 200, "y": 200})
        await asyncio.sleep(0.3)
        return bool(await js("window.__p"))
    return js, alive


async def login_and_wedge(rpc, js, fast_release):
    await rpc("Page.navigate", {"url": "https://www.saucedemo.com/"})
    await asyncio.sleep(3.0)
    await js("""(function(){
        var set = Object.getOwnPropertyDescriptor(
            window.HTMLInputElement.prototype, 'value').set;
        [['#user-name','standard_user'],['#password','secret_sauce']].forEach(function(p){
          var el = document.querySelector(p[0]); set.call(el, p[1]);
          el.dispatchEvent(new Event('input', {bubbles:true}));
          el.dispatchEvent(new Event('change', {bubbles:true}));
        }); return 1; })()""")
    root = (await rpc("DOM.getDocument", {"depth": 0}))["result"]["root"]["nodeId"]
    nid = (await rpc("DOM.querySelector",
                     {"nodeId": root, "selector": "#login-button"}))["result"]["nodeId"]
    c = (await rpc("DOM.getBoxModel", {"nodeId": nid}))["result"]["model"]["content"]
    cx, cy = (c[0] + c[2] + c[4] + c[6]) / 4, (c[1] + c[3] + c[5] + c[7]) / 4
    await rpc("Input.dispatchMouseEvent",
              {"type": "mousePressed", "x": cx, "y": cy, "button": "left",
               "buttons": 1, "clickCount": 1})
    if fast_release:
        # No pause at all: get the release in before the navigation can tear the widget down.
        pass
    else:
        await asyncio.sleep(0.05)
    await rpc("Input.dispatchMouseEvent",
              {"type": "mouseReleased", "x": cx, "y": cy, "button": "left",
               "buttons": 0, "clickCount": 1})
    await asyncio.sleep(3.0)


async def run(label, fast_release, remedies):
    port = PORT + (0 if fast_release else 1)
    profile = PROFILE + ("-fast" if fast_release else "-slow")
    shutil.rmtree(profile, ignore_errors=True)
    proc = subprocess.Popen(
        [CHROME, "--headless=new", "--no-first-run", "--no-default-browser-check",
         "--window-size=1280,900", "--remote-debugging-port=%d" % port,
         "--user-data-dir=%s" % profile, "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(60):
            try:
                targets = [t for t in json.load(urllib.request.urlopen(
                    "http://127.0.0.1:%d/json/list" % port)) if t.get("type") == "page"]
                if targets:
                    break
            except Exception:
                pass
            time.sleep(0.4)
        async with websockets.connect(targets[0]["webSocketDebuggerUrl"], max_size=None) as ws:
            rpc = rpc_of(ws, [0])
            await rpc("Page.enable")
            await rpc("Runtime.enable")
            js, alive = await session(rpc)
            await login_and_wedge(rpc, js, fast_release)
            print("  [%s] ruta=%s  input=%s" % (label, await js("location.pathname"),
                                                "vivo" if await alive() else "MUERTO"))
            if remedies:
                for name, calls in remedies:
                    for method, params in calls:
                        await rpc(method, params)
                    await asyncio.sleep(0.3)
                    ok = await alive()
                    print("      remedio %-34s -> %s" % (name, "VIVO" if ok else "sigue muerto"))
                    if ok:
                        break
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()


REMEDIES = [
    ("mouseReleased suelto", [("Input.dispatchMouseEvent",
                               {"type": "mouseReleased", "x": 5, "y": 5, "button": "left",
                                "buttons": 0, "clickCount": 1})]),
    ("mouseMoved + mouseReleased", [
        ("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": 5, "y": 5, "buttons": 1}),
        ("Input.dispatchMouseEvent", {"type": "mouseReleased", "x": 5, "y": 5,
                                      "button": "left", "buttons": 0, "clickCount": 1})]),
    ("Input.setIgnoreInputEvents(true/false)", [
        ("Input.setIgnoreInputEvents", {"ignore": True}),
        ("Input.setIgnoreInputEvents", {"ignore": False})]),
]


async def main():
    print("A) con pausa entre press y release (como hace el tool)")
    await run("pausa", False, REMEDIES)
    print("B) sin pausa: release inmediato, antes de que la navegacion arranque")
    await run("sin pausa", True, None)


asyncio.run(main())
